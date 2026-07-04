use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dohee_context::Message;
use dohee_core::{Agent, AgentEvent};
use dohee_core as do_core;
use dohee_sandbox::SandboxPolicy;
use dohee_tools::ToolRegistry;
use llama_cpp_2::llama_backend::LlamaBackend;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;

pub struct TuiApp {
    pub messages: Vec<Message>,
    pub status: String,
    pub pending_approval: Option<(String, serde_json::Value, tokio::sync::oneshot::Sender<bool>)>,
    pub list_state: ListState,
    pub input_buf: String,
    pub input_mode: bool, // true: typing prompt, false: viewing/finished
    pub sandbox_desc: String,
    pub tokens_used: usize,
    pub tokens_limit: usize,
    pub finished: bool,
}

impl TuiApp {
    pub fn new(sandbox_desc: String, tokens_limit: usize) -> Self {
        Self {
            messages: Vec::new(),
            status: "Initializing...".to_string(),
            pending_approval: None,
            list_state: ListState::default(),
            input_buf: String::new(),
            input_mode: true,
            sandbox_desc,
            tokens_used: 0,
            tokens_limit,
            finished: false,
        }
    }

    pub fn add_token(&mut self, piece: &str) {
        if let Some(last_msg) = self.messages.last_mut() {
            if last_msg.role == "assistant" {
                last_msg.content.push_str(piece);
                return;
            }
        }
        // If last message is not assistant, create a new one
        self.messages.push(Message {
            role: "assistant".to_string(),
            content: piece.to_string(),
            name: None,
        });
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
}

pub async fn run_tui(
    config: dohee_config::DoheeConfig,
    initial_prompt: Option<String>,
) -> Result<()> {
    // 1. Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 2. Setup channels
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // 3. Initialize state
    let sandbox_desc = config.sandbox_policy.clone();
    let limit = config.ctx_size as usize;
    let app_state = Arc::new(tokio::sync::Mutex::new(TuiApp::new(sandbox_desc, limit)));

    // Set up model parameters clone for background thread
    let config_clone = config.clone();
    let app_state_clone = app_state.clone();

    // 4. Spawn background agent thread
    tokio::spawn(async move {
        let res = async {
            let model_path = config_clone.model_path.as_ref().context("Model path not specified")?;
            let backend = LlamaBackend::init().context("Failed to initialize backend")?;
            
            let gpu_layers = if config_clone.backend == "cpu" { 0 } else { config_clone.gpu_layers };
            let model = dohee_infer::DoheeModel::new(&backend, model_path, gpu_layers)?;

            let sandbox_policy = match config_clone.sandbox_policy.as_str() {
                "ReadOnly" => SandboxPolicy::ReadOnly,
                "DangerFullAccess" => SandboxPolicy::DangerFullAccess,
                _ => SandboxPolicy::WorkspaceWrite {
                    root: std::env::current_dir().unwrap_or_default(),
                },
            };

            let mut registry = ToolRegistry::new();
            registry.register(Arc::new(dohee_tools::ReadFileTool));
            registry.register(Arc::new(dohee_tools::WriteFileTool));
            registry.register(Arc::new(dohee_tools::EditFileTool));
            registry.register(Arc::new(dohee_tools::ListDirTool));
            registry.register(Arc::new(dohee_tools::GrepTool));
            registry.register(Arc::new(dohee_tools::RunShellTool::new(sandbox_policy.clone())));

            let sys_prompt = do_core::system_prompt(&registry.list());
            let mut messages = vec![Message {
                role: "system".to_string(),
                content: sys_prompt,
                name: None,
            }];

            if let Some(prompt) = initial_prompt {
                messages.push(Message {
                    role: "user".to_string(),
                    content: prompt,
                    name: None,
                });
            }

            // Sync initial messages back to TUI
            {
                let mut app = app_state_clone.lock().await;
                for msg in &messages {
                    app.add_message(msg.clone());
                }
            }

            let mut agent = Agent::new(&model, &backend, registry, sandbox_policy);
            agent.temperature = config_clone.temperature;
            agent.seed = config_clone.seed;
            agent.event_tx = Some(event_tx.clone());

            agent.run_turn_loop(&mut messages, config_clone.ctx_size, config_clone.threads).await?;

            if let Some(ref tx) = agent.event_tx {
                let _ = tx.send(AgentEvent::Finished);
            }
            Ok::<(), anyhow::Error>(())
        }.await;

        if let Err(e) = res {
            let mut app = app_state_clone.lock().await;
            app.status = format!("Fatal: {:?}", e);
            app.finished = true;
        }
    });

    // 5. Main TUI render/event loop
    let mut last_tick = std::time::Instant::now();
    let tick_rate = Duration::from_millis(50);

    loop {
        // Draw frame
        {
            let mut app = app_state.lock().await;
            terminal.draw(|f| render_ui(f, &mut app))?;
        }

        // Handle agent events
        while let Ok(event) = event_rx.try_recv() {
            let mut app = app_state.lock().await;
            match event {
                AgentEvent::Token(piece) => {
                    app.add_token(&piece);
                }
                AgentEvent::Status(status) => {
                    app.status = status;
                }
                AgentEvent::ToolRequest { name, args, approve_tx } => {
                    app.status = format!("Pending approval for: {}", name);
                    app.pending_approval = Some((name, args, approve_tx));
                }
                AgentEvent::Finished => {
                    app.status = "Finished task.".to_string();
                    app.finished = true;
                    app.input_mode = false;
                }
            }
        }

        // Handle user input
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                let mut app = app_state.lock().await;

                // 1. Tool approval mode
                if let Some((_name, _args, tx)) = app.pending_approval.take() {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            let _ = tx.send(true);
                            app.status = "Approved execution.".to_string();
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            let _ = tx.send(false);
                            app.status = "Execution denied.".to_string();
                        }
                        _ => {
                            // Put it back if any other key was pressed
                            app.pending_approval = Some((_name, _args, tx));
                        }
                    }
                    continue;
                }

                // 2. Normal navigation/input mode
                match key.code {
                    KeyCode::Esc => {
                        // Exit TUI
                        break;
                    }
                    KeyCode::Up => {
                        if !app.messages.is_empty() {
                            let curr = app.list_state.selected().unwrap_or(0);
                            if curr > 0 {
                                app.list_state.select(Some(curr - 1));
                            }
                        }
                    }
                    KeyCode::Down => {
                        if !app.messages.is_empty() {
                            let curr = app.list_state.selected().unwrap_or(0);
                            if curr < app.messages.len() - 1 {
                                app.list_state.select(Some(curr + 1));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn render_ui(f: &mut Frame, app: &mut TuiApp) {
    // Layout split: Header, Main (Left Conversation, Right Stats), Footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // Main
            Constraint::Length(3), // Footer
        ])
        .split(f.size());

    // 1. Header widget
    let header_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    let header_text = format!(
        " DOHEE (도회) v0.1.0  |  Status: {} ",
        app.status
    );
    let header = Paragraph::new(header_text)
        .block(header_block)
        .style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(header, chunks[0]);

    // Main split: Left Conversation, Right Stats
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70), // Conversation
            Constraint::Percentage(30), // Sidebar
        ])
        .split(chunks[1]);

    // 2. Left Conversation Logger
    let list_items: Vec<ListItem> = app
        .messages
        .iter()
        .map(|msg| {
            let role_span = match msg.role.as_str() {
                "system" => Span::styled("[System]", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
                "user" => Span::styled("[User]", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
                "assistant" => Span::styled("[Assistant]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                "tool" => Span::styled(
                    format!("[Tool: {}]", msg.name.as_deref().unwrap_or("unknown")),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                _ => Span::styled(format!("[{}]", msg.role), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            };

            let mut lines = vec![Line::from(vec![role_span])];
            for l in msg.content.lines() {
                lines.push(Line::from(vec![Span::raw(l)]));
            }
            // Empty space between messages
            lines.push(Line::from(""));

            ListItem::new(lines)
        })
        .collect();

    let list_block = Block::default()
        .borders(Borders::ALL)
        .title(" Conversation History ");
    let list = List::new(list_items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 40, 40))
                .add_modifier(Modifier::ITALIC),
        );
    f.render_stateful_widget(list, main_chunks[0], &mut app.list_state);

    // 3. Right Sidebar (Stats & Sandbox Details)
    let sidebar_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // Token Usage
            Constraint::Min(4),    // Sandbox Details
        ])
        .split(main_chunks[1]);

    // Sidebar: Token Usage
    let token_block = Block::default()
        .borders(Borders::ALL)
        .title(" Context Usage ");
    let token_text = vec![
        Line::from(vec![
            Span::styled("Tokenizer: ", Style::default().fg(Color::DarkGray)),
            Span::raw("llama.cpp built-in"),
        ]),
        Line::from(vec![
            Span::styled("Tokens Limit: ", Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{}", app.tokens_limit)),
        ]),
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Prefix caching enabled", Style::default().fg(Color::Green)),
        ]),
    ];
    let token_para = Paragraph::new(token_text).block(token_block);
    f.render_widget(token_para, sidebar_chunks[0]);

    // Sidebar: Sandbox details
    let sandbox_block = Block::default()
        .borders(Borders::ALL)
        .title(" Sandbox Status ");
    let sandbox_text = vec![
        Line::from(vec![
            Span::styled("Policy: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", app.sandbox_desc), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Kernel LSM: ", Style::default().fg(Color::DarkGray)),
            Span::styled("Landlock V1 (Enforced)", Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Process tree writes isolated to workspace root folder.", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC))),
    ];
    let sandbox_para = Paragraph::new(sandbox_text).block(sandbox_block).wrap(Wrap { trim: true });
    f.render_widget(sandbox_para, sidebar_chunks[1]);

    // 4. Footer (Input Box or Tool Approval Prompt)
    let footer_block = Block::default().borders(Borders::ALL);
    if let Some((ref name, ref args, _)) = app.pending_approval {
        let prompt_text = format!(
            " ⚠️  APPROVAL REQUIRED: Execute tool '{}' with args '{}'?  (y)es / (n)o ",
            name,
            serde_json::to_string(args).unwrap_or_default()
        );
        let footer = Paragraph::new(prompt_text)
            .block(footer_block.title(" Tool Approval ").style(Style::default().fg(Color::LightRed)))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        f.render_widget(footer, chunks[2]);
    } else if app.finished {
        let footer = Paragraph::new(" Task completed. Press [Esc] to exit. Use Up/Down arrows to scroll conversation history. ")
            .block(footer_block.title(" Session Complete ").style(Style::default().fg(Color::Green)))
            .style(Style::default().fg(Color::Green));
        f.render_widget(footer, chunks[2]);
    } else {
        let footer = Paragraph::new(" Agent loop is running... View streamed outputs in the panel above. Use Up/Down to scroll. ")
            .block(footer_block.title(" Interactive Shell ").style(Style::default().fg(Color::DarkGray)))
            .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC));
        f.render_widget(footer, chunks[2]);
    }
}
