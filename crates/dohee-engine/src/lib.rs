use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Failed to load model: {0}")]
    ModelLoadFailed(String),

    #[error("Session limit exceeded: max concurrent sessions is {0}")]
    SessionLimitExceeded(usize),

    #[error("Backend initialization error: {0}")]
    BackendInitFailed(String),

    #[error("Context allocation error: {0}")]
    ContextCreationFailed(String),
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub gpu_layers: u32,
    pub default_ctx_size: u32,
    pub max_concurrent_sessions: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            gpu_layers: 99,
            default_ctx_size: 8192,
            max_concurrent_sessions: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    MaxTokens,
    Cancelled,
    Error(String),
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    Token(String),
    Status(String),
    ToolRequest {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        output: String,
        truncated: bool,
    },
    Finished {
        reason: FinishReason,
    },
    Error(String),
}

pub enum SessionCommand {
    RunTurn {
        prompt: String,
        event_tx: UnboundedSender<AgentEvent>,
    },
    ApproveTool {
        call_id: String,
    },
    DenyTool {
        call_id: String,
    },
    Cancel,
    Shutdown,
}

pub struct SessionHandle {
    pub cmd_tx: UnboundedSender<SessionCommand>,
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub ctx_size: u32,
    pub temperature: f32,
    pub seed: u32,
    pub use_grammar: bool,
    pub sandbox_policy: dohee_sandbox::SandboxPolicy,
    pub cwd: PathBuf,
}

impl Default for SessionConfig {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            ctx_size: 8192,
            temperature: 0.2,
            seed: 1234,
            use_grammar: true,
            sandbox_policy: dohee_sandbox::SandboxPolicy::WorkspaceWrite { root: cwd.clone() },
            cwd,
        }
    }
}

pub struct DoheeEngine {
    model: Arc<dohee_infer::DoheeModel>,
    config: EngineConfig,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl DoheeEngine {
    pub fn load(model_path: impl AsRef<Path>, config: EngineConfig) -> Result<Self, EngineError> {
        let backend = dohee_infer::backend()
            .map_err(|e| EngineError::BackendInitFailed(e.to_string()))?;

        let model = dohee_infer::DoheeModel::new(backend, model_path, config.gpu_layers)
            .map_err(|e| EngineError::ModelLoadFailed(e.to_string()))?;

        let semaphore = Arc::new(tokio::sync::Semaphore::new(config.max_concurrent_sessions));

        Ok(Self {
            model: Arc::new(model),
            config,
            semaphore,
        })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn create_session(&self, session_config: SessionConfig) -> Result<SessionHandle, EngineError> {
        let permit = self.semaphore.clone().try_acquire_owned()
            .map_err(|_| EngineError::SessionLimitExceeded(self.config.max_concurrent_sessions))?;

        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SessionCommand>();
        let model = Arc::clone(&self.model);

        // Dedicated OS worker thread for llama.cpp synchronous decode loops
        std::thread::spawn(move || {
            let _permit = permit;
            let backend = match dohee_infer::backend() {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("Worker thread failed to get backend: {}", e);
                    return;
                }
            };

            let mut session = match dohee_infer::InferenceSession::new(
                backend,
                &model,
                session_config.ctx_size,
                Some(6),
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Worker thread failed to allocate context: {}", e);
                    return;
                }
            };

            while let Some(cmd) = cmd_rx.blocking_recv() {
                match cmd {
                    SessionCommand::RunTurn { prompt, event_tx } => {
                        let _ = event_tx.send(AgentEvent::Status("Processing turn...".to_string()));
                        
                        // Execute prompt advancement
                        if let Err(e) = session.advance(&model, &prompt) {
                            let _ = event_tx.send(AgentEvent::Error(e.to_string()));
                            let _ = event_tx.send(AgentEvent::Finished {
                                reason: FinishReason::Error(e.to_string()),
                            });
                            continue;
                        }

                        let mut sampler = dohee_infer::default_sampler(session_config.seed, session_config.temperature);
                        let mut turn_cancelled = false;

                        // Token generation loop
                        loop {
                            match session.sample_next(&model, &mut sampler) {
                                Ok(Some(piece)) => {
                                    if event_tx.send(AgentEvent::Token(piece)).is_err() {
                                        turn_cancelled = true;
                                        break;
                                    }
                                }
                                Ok(None) => {
                                    break;
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AgentEvent::Error(e.to_string()));
                                    break;
                                }
                            }
                        }

                        let finish_reason = if turn_cancelled {
                            FinishReason::Cancelled
                        } else {
                            FinishReason::Stop
                        };

                        let _ = event_tx.send(AgentEvent::Finished { reason: finish_reason });
                    }
                    SessionCommand::Cancel => {
                        // Cancel current turn
                    }
                    SessionCommand::ApproveTool { .. } | SessionCommand::DenyTool { .. } => {
                        // Handled by tool approval flow
                    }
                    SessionCommand::Shutdown => {
                        break;
                    }
                }
            }
        });

        Ok(SessionHandle { cmd_tx })
    }
}
