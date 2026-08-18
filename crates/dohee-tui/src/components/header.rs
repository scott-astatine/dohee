use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::components::Component;
use crate::app::{TuiApp, AgentMode};
use crate::theme::Theme;

pub struct HeaderComponent {
    theme: Theme,
}

impl HeaderComponent {
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }
}

impl Component for HeaderComponent {
    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()> {
        let mode_str = match app.agent_mode {
            AgentMode::Build => "BUILD",
            AgentMode::Plan => "PLAN",
            AgentMode::Explore => "EXPLORE",
        };

        let header_text = vec![Line::from(vec![
            Span::styled(" DOHEE (도회) v0.1.0 ", Style::default().fg(self.theme.c_cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" │ Model: "),
            Span::styled(&app.model_name, Style::default().fg(self.theme.c_yellow)),
            Span::raw(" │ Mode: "),
            Span::styled(mode_str, Style::default().fg(self.theme.c_green).add_modifier(Modifier::BOLD)),
            Span::raw(" │ Context: "),
            Span::styled(format!("{}/{}", app.tokens_used, app.tokens_limit), Style::default().fg(self.theme.c_magenta)),
            Span::raw(" │ Sandbox: "),
            Span::styled(&app.sandbox_desc, Style::default().fg(self.theme.c_blue)),
        ])];

        let header_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(self.theme.c_subtext))
            .style(Style::default().bg(self.theme.c_header_bg));

        let header = Paragraph::new(header_text).block(header_block);
        f.render_widget(header, rect);
        Ok(())
    }
}
