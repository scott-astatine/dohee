use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use crate::components::Component;
use crate::app::{TuiApp, InputMode};
use crate::action::Action;
use crate::theme::Theme;

pub struct TranscriptComponent {
    theme: Theme,
}

impl TranscriptComponent {
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }
}

impl Component for TranscriptComponent {
    fn handle_key_event(&mut self, key: KeyEvent, app: &mut TuiApp) -> Result<Option<Action>> {
        match app.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('j') | KeyCode::Down => Ok(Some(Action::ScrollDown)),
                KeyCode::Char('k') | KeyCode::Up => Ok(Some(Action::ScrollUp)),
                KeyCode::Char('g') => Ok(Some(Action::ScrollToTop)),
                KeyCode::Char('G') => Ok(Some(Action::ScrollToBottom)),
                _ => Ok(None),
            },
            InputMode::Visual => match key.code {
                KeyCode::Char('j') | KeyCode::Down => Ok(Some(Action::ScrollDown)),
                KeyCode::Char('k') | KeyCode::Up => Ok(Some(Action::ScrollUp)),
                KeyCode::Char('y') => Ok(Some(Action::YankSelection)),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()> {
        let query = app.search_buf.clone();

        let items: Vec<ListItem> = app
            .messages
            .iter()
            .enumerate()
            .map(|(idx, m)| {
                let (prefix, style) = match m.role.as_str() {
                    "user" => ("› You:", self.theme.style_user),
                    "assistant" => ("› Dohee:", self.theme.style_assistant),
                    "system" => ("• System:", self.theme.style_system),
                    _ => ("• Tool:", self.theme.style_tool),
                };

                let mut item_style = Style::default();
                if let (Some(s), Some(e)) = (app.visual_start, app.visual_end) {
                    if idx >= s.min(e) && idx <= s.max(e) {
                        item_style = item_style.bg(Color::Rgb(69, 71, 90)).fg(Color::White);
                    }
                }

                let mut lines = vec![
                    Line::from(Span::styled(prefix, style)),
                ];

                let is_diff = m.role == "tool" || m.content.contains("diff --git") || m.content.contains("--- a/") || m.content.contains("+++ b/");
                let mut in_code_block = false;

                for line in m.content.lines() {
                    lines.push(format_line(line, &mut in_code_block, is_diff, &query, &self.theme));
                }
                lines.push(Line::from("")); // Spacious line gap

                ListItem::new(lines).style(item_style)
            })
            .collect();

        // 1. Draw Messages List
        let list = List::new(items)
            .block(Block::default().borders(Borders::NONE))
            .highlight_style(Style::default().bg(Color::Rgb(45, 45, 60)));
        
        f.render_stateful_widget(list, rect, &mut app.list_state);

        // 2. Overlay Scrollbar Track
        if !app.messages.is_empty() {
            let scrollbar = Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"))
                .track_symbol(Some("░"))
                .thumb_symbol("█")
                .track_style(Style::default().fg(self.theme.c_subtext))
                .thumb_style(Style::default().fg(self.theme.c_cyan));

            let mut scrollbar_state = ScrollbarState::default()
                .content_length(app.messages.len())
                .position(app.list_state.selected().unwrap_or(0));

            f.render_stateful_widget(
                scrollbar,
                rect,
                &mut scrollbar_state,
            );
        }

        Ok(())
    }
}

fn highlight_search(text: &str, query: &str, base_style: Style) -> Line<'static> {
    if query.is_empty() {
        return Line::from(Span::styled(text.to_string(), base_style));
    }
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    
    let mut spans = Vec::new();
    let mut last_idx = 0;
    
    while let Some(start_idx) = text_lower[last_idx..].find(&query_lower) {
        let match_start = last_idx + start_idx;
        let match_end = match_start + query.len();
        
        if match_start > last_idx {
            spans.push(Span::styled(text[last_idx..match_start].to_string(), base_style));
        }
        
        let highlight_style = Style::default()
            .bg(Color::Rgb(249, 226, 175)) // Catppuccin Yellow background
            .fg(Color::Rgb(17, 17, 27))    // dark text
            .add_modifier(Modifier::BOLD);
        spans.push(Span::styled(text[match_start..match_end].to_string(), highlight_style));
        
        last_idx = match_end;
    }
    
    if last_idx < text.len() {
        spans.push(Span::styled(text[last_idx..].to_string(), base_style));
    }
    
    Line::from(spans)
}

fn format_line(
    line: &str,
    in_code_block: &mut bool,
    is_diff: bool,
    query: &str,
    theme: &Theme,
) -> Line<'static> {
    let mut style = Style::default();

    // Markdown code block delimiters
    if line.trim().starts_with("```") {
        *in_code_block = !*in_code_block;
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(theme.c_subtext).add_modifier(Modifier::ITALIC),
        ));
    }

    if *in_code_block {
        // Gray background panel style for code content
        style = style.bg(theme.c_composer_bg).fg(Color::Rgb(205, 214, 244));
        let indented = format!("  {}", line);
        return Line::from(Span::styled(indented, style));
    }

    // Git unified diff lines formatting
    if is_diff {
        if line.starts_with('+') && !line.starts_with("+++") {
            style = style.fg(theme.c_green);
            return Line::from(Span::styled(line.to_string(), style));
        } else if line.starts_with('-') && !line.starts_with("---") {
            style = style.fg(Color::Rgb(243, 139, 168)); // Catppuccin Red
            return Line::from(Span::styled(line.to_string(), style));
        } else if line.starts_with("@@") {
            style = style.fg(theme.c_blue);
            return Line::from(Span::styled(line.to_string(), style));
        }
    }

    // Markdown headers formatting
    if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
        style = style.fg(theme.c_cyan).add_modifier(Modifier::BOLD);
        return Line::from(Span::styled(line.to_string(), style));
    }

    // Bullet points list items formatting
    if line.trim().starts_with("- ") || line.trim().starts_with("* ") {
        style = style.fg(Color::Rgb(205, 214, 244));
        return Line::from(Span::styled(line.to_string(), style));
    }

    // Regular text: apply search text substring highlighting
    if !query.is_empty() {
        highlight_search(line, query, style)
    } else {
        Line::from(Span::styled(line.to_string(), style))
    }
}
