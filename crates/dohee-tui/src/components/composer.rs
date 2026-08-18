use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use crate::components::Component;
use crate::app::{TuiApp, InputMode};
use crate::action::Action;
use crate::theme::Theme;

pub const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/config",
    "/config set",
    "/models",
    "/sessions",
    "/resume",
    "/doctor",
    "/index",
];

pub struct ComposerComponent {
    theme: Theme,
}

impl ComposerComponent {
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }
}

impl Component for ComposerComponent {
    fn handle_key_event(&mut self, key: KeyEvent, app: &mut TuiApp) -> Result<Option<Action>> {
        match app.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => Ok(Some(Action::Exit)),
                KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter => {
                    Ok(Some(Action::SetInputMode(InputMode::Insert)))
                }
                KeyCode::Char('v') | KeyCode::Char('V') => {
                    Ok(Some(Action::SetInputMode(InputMode::Visual)))
                }
                KeyCode::Char('/') => {
                    Ok(Some(Action::SetInputMode(InputMode::Search)))
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Ok(Some(Action::SetInputMode(InputMode::CommandPalette)))
                }
                _ => Ok(None),
            },
            InputMode::Insert => match key.code {
                KeyCode::Esc => Ok(Some(Action::SetInputMode(InputMode::Normal))),
                KeyCode::Tab => {
                    if app.input_buf.starts_with('/') {
                        Ok(Some(Action::CycleAutocomplete))
                    } else {
                        Ok(None)
                    }
                }
                KeyCode::Enter => {
                    if !app.input_buf.is_empty() {
                        let prompt = app.input_buf.clone();
                        app.input_buf.clear();
                        Ok(Some(Action::SubmitPrompt(prompt)))
                    } else {
                        Ok(None)
                    }
                }
                KeyCode::Char(c) => {
                    app.input_buf.push(c);
                    Ok(Some(Action::ResetAutocomplete))
                }
                KeyCode::Backspace => {
                    app.input_buf.pop();
                    Ok(Some(Action::ResetAutocomplete))
                }
                _ => Ok(None),
            },
            InputMode::Search => match key.code {
                KeyCode::Esc => Ok(Some(Action::SetInputMode(InputMode::Normal))),
                KeyCode::Enter => {
                    app.execute_search();
                    Ok(Some(Action::SetInputMode(InputMode::Normal)))
                }
                KeyCode::Char(c) => {
                    app.search_buf.push(c);
                    Ok(None)
                }
                KeyCode::Backspace => {
                    app.search_buf.pop();
                    Ok(None)
                }
                _ => Ok(None),
            },
            InputMode::CommandPalette => match key.code {
                KeyCode::Esc => Ok(Some(Action::SetInputMode(InputMode::Normal))),
                KeyCode::Enter => {
                    let selected = app.command_palette_selected;
                    let action = match selected {
                        0 => Action::SetAgentMode(crate::AgentMode::Build),
                        1 => Action::SetAgentMode(crate::AgentMode::Plan),
                        2 => Action::SetAgentMode(crate::AgentMode::Explore),
                        _ => Action::Noop,
                    };
                    Ok(Some(action))
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.command_palette_selected = (app.command_palette_selected + 1) % 3;
                    Ok(None)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.command_palette_selected = (app.command_palette_selected + 2) % 3;
                    Ok(None)
                }
                _ => Ok(None),
            },
            InputMode::Approval => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Ok(Some(Action::ApproveTool(true))),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Ok(Some(Action::ApproveTool(false))),
                _ => Ok(None),
            },
            InputMode::Visual => match key.code {
                KeyCode::Esc => Ok(Some(Action::SetInputMode(InputMode::Normal))),
                _ => Ok(None),
            },
        }
    }

    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()> {
        let mode_title = match app.input_mode {
            InputMode::Normal => " [NORMAL MODE - 'i' Chat / 'v' Visual / '/' Search] ".to_string(),
            InputMode::Insert => {
                if app.input_buf.starts_with('/') {
                    let matches: Vec<&str> = SLASH_COMMANDS
                        .iter()
                        .filter(|cmd| cmd.starts_with(&app.input_buf))
                        .cloned()
                        .collect();
                    if !matches.is_empty() {
                        format!(" [INSERT MODE - Suggestions: {}] ", matches.join(" │ "))
                    } else {
                        " [INSERT MODE - Slash Command] ".to_string()
                    }
                } else {
                    " [INSERT MODE - Type prompt and press Enter] ".to_string()
                }
            }
            InputMode::Visual => " [VISUAL MODE - 'j/k' Select / 'y' Yank] ".to_string(),
            InputMode::Search => " [SEARCH MODE - Enter query] ".to_string(),
            InputMode::CommandPalette => " [COMMAND PALETTE] ".to_string(),
            InputMode::Approval => " [APPROVAL REQUIRED - 'y' Approve / 'n' Deny] ".to_string(),
        };

        let (mode_color, indicator) = match app.input_mode {
            InputMode::Normal => (self.theme.c_green, "› "),
            InputMode::Insert => (self.theme.c_blue, "› "),
            InputMode::Visual => (self.theme.c_magenta, "› "),
            InputMode::Search => (self.theme.c_yellow, " /"),
            InputMode::CommandPalette => (self.theme.c_cyan, " :"),
            InputMode::Approval => (ratatui::style::Color::Red, "› "),
        };

        let input_display = if app.input_mode == InputMode::Search {
            format!("{}{}", indicator, app.search_buf)
        } else {
            format!("{}{}", indicator, app.input_buf)
        };

        let input_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(self.theme.c_subtext))
            .title(mode_title)
            .style(Style::default().bg(self.theme.c_composer_bg));

        let input = Paragraph::new(input_display)
            .style(Style::default().fg(mode_color))
            .block(input_block)
            .wrap(Wrap { trim: false });
            
        f.render_widget(input, rect);
        Ok(())
    }
}
