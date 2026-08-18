use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::Style,
    widgets::Paragraph,
    Frame,
};
use crate::components::Component;
use crate::app::TuiApp;
use crate::theme::Theme;

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub struct StatusBarComponent {
    theme: Theme,
    spinner_tick: usize,
}

impl StatusBarComponent {
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
            spinner_tick: 0,
        }
    }
}

impl Component for StatusBarComponent {
    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()> {
        let is_running = app.status != "Ready" && !app.finished;
        let spinner_char = if is_running {
            let symbol = SPINNER[self.spinner_tick % SPINNER.len()];
            self.spinner_tick += 1;
            format!("{} ", symbol)
        } else {
            "".to_string()
        };

        let status_bar = Paragraph::new(format!("  Status: {}{}  │  q: Exit  │  Ctrl+p: Commands", spinner_char, app.status))
            .style(Style::default().bg(self.theme.c_footer_bg).fg(self.theme.c_subtext));
        f.render_widget(status_bar, rect);
        Ok(())
    }
}
