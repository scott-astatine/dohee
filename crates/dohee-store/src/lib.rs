//! `dohee-store` implements session persistence using SQLite (via `rusqlite`).
//! It saves active chat histories, model profiles, and token logs to allow resuming past coding sessions after process restarts.

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
