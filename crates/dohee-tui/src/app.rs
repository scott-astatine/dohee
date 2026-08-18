use anyhow::Result;
use dohee_context::Message;
use ratatui::widgets::ListState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Insert,
    Visual,
    Approval,
    Search,
    CommandPalette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Build,
    Plan,
    Explore,
}

#[derive(Debug, Clone)]
pub enum TuiCommand {
    SubmitPrompt {
        prompt: String,
        messages: Vec<Message>,
    },
    UpdateConfig {
        temp: Option<f32>,
        seed: Option<u32>,
        ctx_size: Option<u32>,
        sandbox_policy: Option<dohee_sandbox::SandboxPolicy>,
    },
}

pub struct TuiApp {
    pub messages: Vec<Message>,
    pub status: String,
    pub input_mode: InputMode,
    pub agent_mode: AgentMode,
    pub list_state: ListState,
    pub input_buf: String,
    pub search_buf: String,
    pub search_match_idx: usize,
    pub search_matches: Vec<usize>,
    pub visual_start: Option<usize>,
    pub visual_end: Option<usize>,
    pub model_name: String,
    pub sandbox_desc: String,
    pub tokens_used: usize,
    pub tokens_limit: usize,
    pub pending_approval: Option<(String, serde_json::Value, tokio::sync::oneshot::Sender<bool>)>,
    pub command_palette_selected: usize,
    pub finished: bool,
    
    // Autocomplete state
    pub autocomplete_prefix: Option<String>,
    pub autocomplete_matches: Vec<String>,
    pub autocomplete_idx: usize,
}

impl TuiApp {
    pub fn new(model_name: String, sandbox_desc: String, tokens_limit: usize) -> Self {
        Self {
            messages: Vec::new(),
            status: "Ready".to_string(),
            input_mode: InputMode::Normal,
            agent_mode: AgentMode::Build,
            list_state: ListState::default(),
            input_buf: String::new(),
            search_buf: String::new(),
            search_match_idx: 0,
            search_matches: Vec::new(),
            visual_start: None,
            visual_end: None,
            model_name,
            sandbox_desc,
            tokens_used: 0,
            tokens_limit,
            pending_approval: None,
            command_palette_selected: 0,
            finished: false,
            autocomplete_prefix: None,
            autocomplete_matches: Vec::new(),
            autocomplete_idx: 0,
        }
    }

    pub fn cycle_autocomplete(&mut self) {
        if self.autocomplete_prefix.is_none() {
            self.autocomplete_prefix = Some(self.input_buf.clone());
            let matches: Vec<String> = crate::components::composer::SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(&self.input_buf))
                .map(|cmd| cmd.to_string())
                .collect();
            if matches.is_empty() {
                self.autocomplete_prefix = None;
                return;
            }
            self.autocomplete_matches = matches;
            self.autocomplete_idx = 0;
        }

        if !self.autocomplete_matches.is_empty() {
            self.input_buf = self.autocomplete_matches[self.autocomplete_idx].clone();
            self.autocomplete_idx = (self.autocomplete_idx + 1) % self.autocomplete_matches.len();
        }
    }

    pub fn reset_autocomplete(&mut self) {
        self.autocomplete_prefix = None;
        self.autocomplete_matches.clear();
        self.autocomplete_idx = 0;
    }

    pub fn add_token(&mut self, piece: &str) {
        if let Some(last_msg) = self.messages.last_mut() {
            if last_msg.role == "assistant" {
                last_msg.content.push_str(piece);
                self.tokens_used += piece.len() / 4;
                return;
            }
        }
        self.messages.push(Message {
            role: "assistant".to_string(),
            content: piece.to_string(),
            name: None,
        });
        self.tokens_used += piece.len() / 4;
        self.scroll_to_bottom();
    }

    pub fn add_message(&mut self, msg: Message) {
        self.messages.push(msg);
        self.scroll_to_bottom();
    }

    pub fn scroll_to_bottom(&mut self) {
        if !self.messages.is_empty() {
            self.list_state.select(Some(self.messages.len() - 1));
        }
    }

    pub fn yank_visual_selection(&mut self) -> Result<()> {
        if let (Some(start), Some(end)) = (self.visual_start, self.visual_end) {
            let min = start.min(end);
            let max = start.max(end);
            let selected_text: Vec<String> = self.messages[min..=max.min(self.messages.len().saturating_sub(1))]
                .iter()
                .map(|m| format!("[{}]: {}", m.role, m.content))
                .collect();
            let joined = selected_text.join("\n\n");
            
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(joined);
                self.status = format!("Yanked {} message(s) to clipboard", max - min + 1);
            }
        }
        self.input_mode = InputMode::Normal;
        self.visual_start = None;
        self.visual_end = None;
        Ok(())
    }

    pub fn execute_search(&mut self) {
        self.search_matches.clear();
        if self.search_buf.is_empty() {
            return;
        }
        for (i, msg) in self.messages.iter().enumerate() {
            if msg.content.to_lowercase().contains(&self.search_buf.to_lowercase()) {
                self.search_matches.push(i);
            }
        }
        if !self.search_matches.is_empty() {
            self.search_match_idx = 0;
            self.list_state.select(Some(self.search_matches[0]));
            self.status = format!("Match 1/{}", self.search_matches.len());
        } else {
            self.status = "No matches found".to_string();
        }
    }

    pub fn next_search_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_match_idx = (self.search_match_idx + 1) % self.search_matches.len();
        let target = self.search_matches[self.search_match_idx];
        self.list_state.select(Some(target));
        self.status = format!("Match {}/{}", self.search_match_idx + 1, self.search_matches.len());
    }

    pub async fn handle_slash_command(
        &mut self,
        cmd_raw: &str,
        ui_cmd_tx: &tokio::sync::mpsc::UnboundedSender<TuiCommand>,
    ) -> Result<()> {
        let trimmed = cmd_raw.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        let cmd = parts[0];
        match cmd {
            "/help" => {
                let help_text = "\
Available TUI Commands:
  /help                  - Show this help message
  /config                - Show current configuration
  /config set <key> <val>- Set configuration parameter (temp, seed, ctx_size, sandbox)
  /models                - List GGUF models in models/ directory
  /sessions              - List past chat sessions from DB
  /resume <session_id>   - Resume a past chat session from DB
  /doctor                - Run system diagnostics (hardware, Vulkan/CUDA, sandbox)
  /index                 - Build/refresh AST symbol index for workspace
";
                self.add_message(Message {
                    role: "system".to_string(),
                    content: help_text.to_string(),
                    name: None,
                });
            }
            "/config" => {
                if parts.len() >= 4 && parts[1] == "set" {
                    let key = parts[2];
                    let val = parts[3];
                    
                    match key {
                        "temperature" | "temp" => {
                            if let Ok(t) = val.parse::<f32>() {
                                let _ = ui_cmd_tx.send(TuiCommand::UpdateConfig {
                                    temp: Some(t),
                                    seed: None,
                                    ctx_size: None,
                                    sandbox_policy: None,
                                });
                                self.status = format!("Updated temperature to {}", t);
                                self.add_message(Message {
                                    role: "system".to_string(),
                                    content: format!("Configuration updated: temperature = {}", t),
                                    name: None,
                                });
                            }
                        }
                        "seed" => {
                            if let Ok(s) = val.parse::<u32>() {
                                let _ = ui_cmd_tx.send(TuiCommand::UpdateConfig {
                                    temp: None,
                                    seed: Some(s),
                                    ctx_size: None,
                                    sandbox_policy: None,
                                });
                                self.status = format!("Updated seed to {}", s);
                                self.add_message(Message {
                                    role: "system".to_string(),
                                    content: format!("Configuration updated: seed = {}", s),
                                    name: None,
                                });
                            }
                        }
                        "ctx_size" | "context" => {
                            if let Ok(c) = val.parse::<u32>() {
                                let _ = ui_cmd_tx.send(TuiCommand::UpdateConfig {
                                    temp: None,
                                    seed: None,
                                    ctx_size: Some(c),
                                    sandbox_policy: None,
                                });
                                self.tokens_limit = c as usize;
                                self.status = format!("Updated context size limit to {}", c);
                                self.add_message(Message {
                                    role: "system".to_string(),
                                    content: format!("Configuration updated: context size limit = {}", c),
                                    name: None,
                                });
                            }
                        }
                        "sandbox" => {
                            let policy = match val {
                                "ReadOnly" => Some(dohee_sandbox::SandboxPolicy::ReadOnly),
                                "DangerFullAccess" => Some(dohee_sandbox::SandboxPolicy::DangerFullAccess),
                                _ => Some(dohee_sandbox::SandboxPolicy::WorkspaceWrite {
                                    root: std::env::current_dir().unwrap_or_default(),
                                }),
                            };
                            if let Some(sp) = policy {
                                self.sandbox_desc = val.to_string();
                                let _ = ui_cmd_tx.send(TuiCommand::UpdateConfig {
                                    temp: None,
                                    seed: None,
                                    ctx_size: None,
                                    sandbox_policy: Some(sp),
                                });
                                self.status = format!("Updated sandbox policy to {}", val);
                                self.add_message(Message {
                                    role: "system".to_string(),
                                    content: format!("Configuration updated: sandbox policy = {}", val),
                                    name: None,
                                });
                            }
                        }
                        _ => {
                            self.add_message(Message {
                                role: "system".to_string(),
                                content: format!("Unknown configuration property '{}'. Use temperature, seed, ctx_size, or sandbox.", key),
                                name: None,
                            });
                        }
                    }
                } else {
                    let config_text = format!(
                        "Current Configuration:\n  Model Name: {}\n  Sandbox Policy: {}\n  Token Limit: {}\n  Tokens Used: {}",
                        self.model_name, self.sandbox_desc, self.tokens_limit, self.tokens_used
                    );
                    self.add_message(Message {
                        role: "system".to_string(),
                        content: config_text,
                        name: None,
                    });
                }
            }
            "/models" => {
                let models_dir = std::path::Path::new("models");
                if models_dir.exists() {
                    let mut list = String::new();
                    list.push_str("Available GGUF models:\n");
                    if let Ok(entries) = std::fs::read_dir(models_dir) {
                        for entry in entries {
                            if let Ok(entry) = entry {
                                let path = entry.path();
                                if path.is_file() && path.extension().map_or(false, |e| e == "gguf") {
                                    list.push_str(&format!("  - {}\n", path.file_name().unwrap().to_string_lossy()));
                                }
                            }
                        }
                    }
                    self.add_message(Message {
                        role: "system".to_string(),
                        content: list,
                        name: None,
                    });
                } else {
                    self.add_message(Message {
                        role: "system".to_string(),
                        content: "Error: models/ directory does not exist.".to_string(),
                        name: None,
                    });
                }
            }
            "/sessions" => {
                let home = std::env::var("HOME").ok();
                let config_dir = home.map(|h| std::path::PathBuf::from(h).join(".config/dohee"));
                let db_path = config_dir.unwrap_or_else(|| std::path::PathBuf::from(".")).join("sessions.db");

                if let Ok(store) = dohee_store::Store::open(db_path) {
                    if let Ok(sessions) = store.list_sessions() {
                        let mut msg = String::new();
                        msg.push_str("Past Agent Sessions:\n");
                        for (id, created) in sessions {
                            msg.push_str(&format!("  - ID: {} (Created: {})\n", id, created));
                        }
                        self.add_message(Message {
                            role: "system".to_string(),
                            content: msg,
                            name: None,
                        });
                    } else {
                        self.add_message(Message {
                            role: "system".to_string(),
                            content: "Failed to read sessions from database.".to_string(),
                            name: None,
                        });
                    }
                } else {
                    self.add_message(Message {
                        role: "system".to_string(),
                        content: "Failed to open sessions database.".to_string(),
                        name: None,
                    });
                }
            }
            "/resume" => {
                if parts.len() < 2 {
                    self.add_message(Message {
                        role: "system".to_string(),
                        content: "Usage: /resume <session_id>".to_string(),
                        name: None,
                    });
                } else {
                    let session_id = parts[1];
                    let home = std::env::var("HOME").ok();
                    let config_dir = home.map(|h| std::path::PathBuf::from(h).join(".config/dohee"));
                    let db_path = config_dir.unwrap_or_else(|| std::path::PathBuf::from(".")).join("sessions.db");

                    if let Ok(store) = dohee_store::Store::open(db_path) {
                        if let Ok(Some(session)) = store.load_session(session_id) {
                            self.messages = session.messages;
                            self.scroll_to_bottom();
                            self.status = format!("Resumed session '{}'", session_id);
                        } else {
                            self.add_message(Message {
                                role: "system".to_string(),
                                content: format!("Session '{}' not found.", session_id),
                                name: None,
                            });
                        }
                    } else {
                        self.add_message(Message {
                            role: "system".to_string(),
                            content: "Failed to load session database.".to_string(),
                            name: None,
                        });
                    }
                }
            }
            "/doctor" => {
                let mut report = String::new();
                report.push_str("Dohee Diagnostics System Doctor:\n");
                report.push_str(&format!("  - OS Platform: {} ({})\n", std::env::consts::OS, std::env::consts::ARCH));

                #[cfg(feature = "cuda")]
                report.push_str("  - Hardware Acceleration: CUDA (Enabled)\n");
                #[cfg(feature = "vulkan")]
                report.push_str("  - Hardware Acceleration: Vulkan (Enabled)\n");
                #[cfg(not(any(feature = "cuda", feature = "vulkan")))]
                report.push_str("  - Hardware Acceleration: CPU (Fall-back)\n");

                report.push_str(&format!("  - Current Sandbox Policy: {}\n", self.sandbox_desc));
                report.push_str(&format!("  - Token Limit: {}\n", self.tokens_limit));

                self.add_message(Message {
                    role: "system".to_string(),
                    content: report,
                    name: None,
                });
            }
            "/index" => {
                self.status = "Building AST index in background...".to_string();
                self.add_message(Message {
                    role: "system".to_string(),
                    content: "Triggered AST indexing. Refreshing tree-sitter workspace cache.".to_string(),
                    name: None,
                });
                
                if let Ok(exe) = std::env::current_exe() {
                    tokio::spawn(async move {
                        let _ = tokio::process::Command::new(exe)
                            .arg("index")
                            .output()
                            .await;
                    });
                }
            }
            _ => {
                self.add_message(Message {
                    role: "system".to_string(),
                    content: format!("Unknown slash command: {}. Type /help for assistance.", cmd),
                    name: None,
                });
            }
        }
        Ok(())
    }
}
