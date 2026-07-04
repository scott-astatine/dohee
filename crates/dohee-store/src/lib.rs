use anyhow::{Context, Result};
use dohee_context::{Message, Session};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open SQLite database")?;
        let mut store = Self { conn };
        store.init_db().context("Failed to initialize database schema")?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory SQLite database")?;
        let mut store = Self { conn };
        store.init_db().context("Failed to initialize database schema")?;
        Ok(store)
    }

    fn init_db(&mut self) -> Result<()> {
        // Create sessions table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                compaction_generation INTEGER NOT NULL DEFAULT 0
            )",
            [],
        ).context("Failed to create sessions table")?;

        // Create messages table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                name TEXT,
                timestamp INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        ).context("Failed to create messages table")?;

        Ok(())
    }

    pub fn save_session(&mut self, session_id: &str, session: &Session) -> Result<()> {
        let tx = self.conn.transaction().context("Failed to begin database transaction")?;

        // Save session record
        tx.execute(
            "INSERT OR REPLACE INTO sessions (id, created_at, compaction_generation) VALUES (?1, ?2, ?3)",
            params![session_id, session.created_at as i64, session.compaction_generation],
        ).context("Failed to save session metadata")?;

        // Clear existing messages
        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session_id],
        ).context("Failed to purge old messages")?;

        // Insert fresh message batch
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        for msg in &session.messages {
            tx.execute(
                "INSERT INTO messages (session_id, role, content, name, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    session_id,
                    msg.role,
                    msg.content,
                    msg.name,
                    now
                ],
            ).context("Failed to insert message")?;
        }

        tx.commit().context("Failed to commit database transaction")?;
        Ok(())
    }

    pub fn load_session(&self, session_id: &str) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT created_at, compaction_generation FROM sessions WHERE id = ?1"
        ).context("Failed to prepare session SELECT statement")?;

        let mut session_rows = stmt.query_map(params![session_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, u32>(1)?))
        })?;

        let Some(row_res) = session_rows.next() else {
            return Ok(None);
        };
        let (created_at, compaction_generation) = row_res.context("Failed to parse session metadata row")?;

        // Load messages ordered by id
        let mut stmt = self.conn.prepare(
            "SELECT role, content, name FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        ).context("Failed to prepare messages SELECT statement")?;

        let message_rows = stmt.query_map(params![session_id], |row| {
            Ok(Message {
                role: row.get(0)?,
                content: row.get(1)?,
                name: row.get(2)?,
            })
        })?;

        let mut messages = Vec::new();
        for msg_res in message_rows {
            messages.push(msg_res.context("Failed to parse message row")?);
        }

        Ok(Some(Session {
            messages,
            created_at: created_at as u64,
            compaction_generation,
        }))
    }

    pub fn list_sessions(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at FROM sessions ORDER BY created_at DESC"
        ).context("Failed to prepare list SELECT statement")?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.context("Failed to parse session list row")?);
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_save_load_list() -> Result<()> {
        let mut store = Store::open_in_memory()?;

        let session = Session {
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "sys prompt".to_string(),
                    name: None,
                },
                Message {
                    role: "user".to_string(),
                    content: "hello".to_string(),
                    name: None,
                },
            ],
            created_at: 1000,
            compaction_generation: 2,
        };

        // 1. Save session
        store.save_session("session-abc", &session)?;

        // 2. List sessions
        let list = store.list_sessions()?;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "session-abc");
        assert_eq!(list[0].1, 1000);

        // 3. Load session
        let loaded = store.load_session("session-abc")?;
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.created_at, 1000);
        assert_eq!(loaded.compaction_generation, 2);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "system");
        assert_eq!(loaded.messages[1].content, "hello");

        // 4. Load nonexistent session
        let nonexistent = store.load_session("session-xyz")?;
        assert!(nonexistent.is_none());

        Ok(())
    }
}
