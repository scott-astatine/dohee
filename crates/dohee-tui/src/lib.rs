//! # `dohee-tui`
//!
//! A borderless, true-color Terminal User Interface (TUI) for the Dohee local-first AI coding agent.
//! Inspired by `codex-cli`, it implements a Component-based Model-View-Update (MVU) architecture.
//!
//! ## Architecture Overview
//!
//! The subcrate is structured into several modular layers:
//! - [`terminal`]: RAII-safe raw mode and screen buffer initialization (`TerminalGuard`).
//! - [`action`]: Centralized `Action` enum that models user inputs and background agent events.
//! - [`app`]: The core `TuiApp` state coordinator acting as the Model and the Action transition loop.
//! - [`theme`]: Catppuccin Mocha-themed true-color constants and paragraph styling templates.
//! - [`worker`]: Asynchronous background runner wrapper coordinating database lookups and blocking llama.cpp turns.
//! - [`components`]: Visual widget sections (Header, Transcript, Composer, StatusBar, Popups) implementing the common `Component` trait.

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use dohee_context::Message;
use ratatui::{
    layout::{Constraint, Direction, Layout},
};
use std::sync::Arc;
use std::time::Duration;

pub mod action;
pub mod app;
pub mod components;
pub mod terminal;
pub mod theme;
pub mod worker;

pub use app::{AgentMode, InputMode, TuiApp, TuiCommand};
use action::Action;
use components::Component;

#[derive(Debug, Clone)]
pub enum TuiEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
}

pub async fn run_tui(
    config: dohee_config::DoheeConfig,
    initial_prompt: Option<String>,
) -> Result<()> {
    // 1. Initialize RAII Terminal Guard
    let mut guard = terminal::TerminalGuard::new()?;

    // 2. Load LLM Backend singleton
    let backend_tup = dohee_infer::backend().context("Failed to get backend")?;
    let gpu_layers = if config.backend == "cpu" { 0 } else { config.gpu_layers };
    let model_path = config.model_path.clone().unwrap_or_else(|| std::path::PathBuf::from("models/Gemma-4e3bu-ag.gguf"));
    let model = dohee_infer::DoheeModel::new(backend_tup, &model_path, gpu_layers)
        .context("Failed to load model")?;
        
    let model_ref: &'static dohee_infer::DoheeModel = Box::leak(Box::new(model));
    let backend_ref = backend_tup; // &'static LlamaBackend

    // 3. Register standard agent tools
    let mut registry = dohee_tools::ToolRegistry::new();
    let sandbox_policy = match config.sandbox_policy.as_str() {
        "ReadOnly" => dohee_sandbox::SandboxPolicy::ReadOnly,
        "DangerFullAccess" => dohee_sandbox::SandboxPolicy::DangerFullAccess,
        _ => dohee_sandbox::SandboxPolicy::WorkspaceWrite {
            root: std::env::current_dir().unwrap_or_default(),
        },
    };
    registry.register(std::sync::Arc::new(dohee_tools::ReadFileTool));
    registry.register(std::sync::Arc::new(dohee_tools::WriteFileTool));
    registry.register(std::sync::Arc::new(dohee_tools::EditFileTool));
    registry.register(std::sync::Arc::new(dohee_tools::ListDirTool));
    registry.register(std::sync::Arc::new(dohee_tools::GrepTool));
    registry.register(std::sync::Arc::new(dohee_tools::RunShellTool::new(sandbox_policy.clone())));
    registry.register(std::sync::Arc::new(dohee_tools::ListSymbolsTool));
    registry.register(std::sync::Arc::new(dohee_tools::FindDefinitionTool));

    // 4. Start Background Worker
    let model_name = config
        .model_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "qwen2.5".to_string());
    let sandbox_desc = config.sandbox_policy.clone();
    let limit = config.ctx_size as usize;
    
    let mut worker = worker::AgentWorker::spawn(config, registry, model_ref, backend_ref)
        .context("Failed to spawn background agent worker")?;
    let app_state = Arc::new(tokio::sync::Mutex::new(TuiApp::new(model_name, sandbox_desc, limit)));

    // 5. Spawn keyboard/resize event reader thread
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
    std::thread::spawn(move || {
        loop {
            if let Ok(true) = event::poll(Duration::from_millis(50)) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if event_tx.send(TuiEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Resize(w, h)) => {
                        if event_tx.send(TuiEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    // 6. Initialize UI Components
    let mut header_comp = components::header::HeaderComponent::new();
    let mut transcript_comp = components::transcript::TranscriptComponent::new();
    let mut composer_comp = components::composer::ComposerComponent::new();
    let mut status_bar_comp = components::status_bar::StatusBarComponent::new();
    let mut popups_comp = components::popups::PopupsComponent::new();

    // 7. Handle initial prompt if supplied
    if let Some(prompt) = initial_prompt {
        let mut app = app_state.lock().await;
        app.add_message(Message {
            role: "user".to_string(),
            content: prompt.clone(),
            name: None,
        });
        app.status = "Agent running...".to_string();
        let messages_copy = app.messages.clone();
        let _ = worker.ui_cmd_tx.send(TuiCommand::SubmitPrompt {
            prompt,
            messages: messages_copy,
        });
    }

    let mut current_tool_approve_tx: Option<tokio::sync::oneshot::Sender<bool>> = None;

    // 8. Main Event Pump
    loop {
        {
            let mut app = app_state.lock().await;
            guard.terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(0)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(8),
                        Constraint::Length(3),
                        Constraint::Length(1),
                    ])
                    .split(f.size());

                let _ = header_comp.draw(f, chunks[0], &mut app);
                let _ = transcript_comp.draw(f, chunks[1], &mut app);
                let _ = composer_comp.draw(f, chunks[2], &mut app);
                let _ = status_bar_comp.draw(f, chunks[3], &mut app);
                let _ = popups_comp.draw(f, f.size(), &mut app);
            })?;

            if app.finished {
                break;
            }
        }

        tokio::select! {
            Some(tui_event) = event_rx.recv() => {
                match tui_event {
                    TuiEvent::Key(key) => {
                        let mut app = app_state.lock().await;

                        // Pass key events down to components depending on mode
                        let mut action = None;
                        if app.input_mode == InputMode::CommandPalette {
                            action = popups_comp.handle_key_event(key, &mut app)?;
                        }
                        if action.is_none() {
                            action = composer_comp.handle_key_event(key, &mut app)?;
                        }
                        if action.is_none() {
                            action = transcript_comp.handle_key_event(key, &mut app)?;
                        }

                        if let Some(act) = action {
                            dispatch_action(act, &mut app, &mut worker, &mut current_tool_approve_tx).await?;
                        }
                    }
                    TuiEvent::Resize(_w, _h) => {
                        // Automatically updates layout on next loop draw
                    }
                }
            }
            Some(agent_event) = worker.agent_rx.recv() => {
                let mut app = app_state.lock().await;
                match agent_event {
                    dohee_core::AgentEvent::Token(tok) => {
                        app.add_token(&tok);
                    }
                    dohee_core::AgentEvent::Status(stat) => {
                        app.status = stat;
                    }
                    dohee_core::AgentEvent::ToolRequest { name, args, approve_tx } => {
                        app.status = format!("Approval requested for tool '{}'", name);
                        app.input_mode = InputMode::Approval;
                        current_tool_approve_tx = Some(approve_tx);
                        app.pending_approval = Some((name, args, tokio::sync::oneshot::channel::<bool>().0));
                    }
                    dohee_core::AgentEvent::ToolResult { name, output } => {
                        app.status = format!("Tool '{}' finished execution.", name);
                        app.add_message(Message {
                            role: "tool".to_string(),
                            content: format!("Tool output:\n{}", output),
                            name: Some(name),
                        });
                    }
                    dohee_core::AgentEvent::Finished => {
                        app.status = "Ready".to_string();
                        app.input_mode = InputMode::Normal;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn dispatch_action(
    action: Action,
    app: &mut TuiApp,
    worker: &mut worker::AgentWorker,
    current_tool_approve_tx: &mut Option<tokio::sync::oneshot::Sender<bool>>,
) -> Result<()> {
    match action {
        Action::SubmitPrompt(prompt) => {
            if prompt.starts_with('/') {
                app.handle_slash_command(&prompt, &worker.ui_cmd_tx).await?;
            } else {
                app.add_message(Message {
                    role: "user".to_string(),
                    content: prompt.clone(),
                    name: None,
                });
                app.status = "Agent running...".to_string();
                app.input_mode = InputMode::Normal;
                let messages_copy = app.messages.clone();
                let _ = worker.ui_cmd_tx.send(TuiCommand::SubmitPrompt {
                    prompt,
                    messages: messages_copy,
                });
            }
        }
        Action::ApproveTool(approved) => {
            if let Some(tx) = current_tool_approve_tx.take() {
                let _ = tx.send(approved);
            }
            app.status = if approved { "Approved tool execution".to_string() } else { "Denied tool execution".to_string() };
            app.input_mode = InputMode::Normal;
            app.pending_approval = None;
        }
        Action::UpdateConfig { temp, seed, ctx_size, sandbox_policy } => {
            let _ = worker.ui_cmd_tx.send(TuiCommand::UpdateConfig {
                temp,
                seed,
                ctx_size,
                sandbox_policy,
            });
        }
        Action::ScrollUp => {
            if app.input_mode == InputMode::Visual {
                if let Some(curr) = app.visual_end {
                    if curr > 0 {
                        app.visual_end = Some(curr - 1);
                        app.list_state.select(Some(curr - 1));
                    }
                }
            } else if !app.messages.is_empty() {
                let curr = app.list_state.selected().unwrap_or(0);
                if curr > 0 {
                    app.list_state.select(Some(curr - 1));
                }
            }
        }
        Action::ScrollDown => {
            if app.input_mode == InputMode::Visual {
                if let Some(curr) = app.visual_end {
                    if curr < app.messages.len() - 1 {
                        app.visual_end = Some(curr + 1);
                        app.list_state.select(Some(curr + 1));
                    }
                }
            } else if !app.messages.is_empty() {
                let curr = app.list_state.selected().unwrap_or(0);
                if curr < app.messages.len() - 1 {
                    app.list_state.select(Some(curr + 1));
                }
            }
        }
        Action::ScrollToTop => {
            if !app.messages.is_empty() {
                app.list_state.select(Some(0));
            }
        }
        Action::ScrollToBottom => {
            app.scroll_to_bottom();
        }
        Action::YankSelection => {
            app.yank_visual_selection()?;
        }
        Action::CycleAutocomplete => {
            app.cycle_autocomplete();
        }
        Action::ResetAutocomplete => {
            app.reset_autocomplete();
        }
        Action::ToggleCommandPalette => {
            app.input_mode = InputMode::CommandPalette;
        }
        Action::SetAgentMode(mode) => {
            app.agent_mode = mode;
            app.input_mode = InputMode::Normal;
        }
        Action::SetInputMode(mode) => {
            app.input_mode = mode;
            if mode == InputMode::Visual {
                let sel = app.list_state.selected().unwrap_or(0);
                app.visual_start = Some(sel);
                app.visual_end = Some(sel);
            } else {
                app.visual_start = None;
                app.visual_end = None;
            }
        }
        Action::Exit => {
            app.finished = true;
        }
        _ => {}
    }
    Ok(())
}
