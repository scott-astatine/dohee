use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
    Frame,
};
use crate::components::Component;
use crate::app::{TuiApp, InputMode};
use crate::theme::Theme;

pub struct PopupsComponent {
    theme: Theme,
}

impl PopupsComponent {
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }
}

impl Component for PopupsComponent {
    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()> {
        if app.input_mode == InputMode::CommandPalette {
            let area = centered_rect(60, 40, rect);
            f.render_widget(Clear, area);

            let options = vec![
                ListItem::new("1. Set Agent Mode -> BUILD (Read & Write Tools)"),
                ListItem::new("2. Set Agent Mode -> PLAN (Read-Only Planning)"),
                ListItem::new("3. Set Agent Mode -> EXPLORE (Fast Search Only)"),
            ];

            let palette = List::new(options)
                .block(Block::default().borders(Borders::ALL).title(" Command Palette (Use j/k to select, Enter to apply) "))
                .highlight_style(Style::default().fg(Color::Black).bg(self.theme.c_cyan));
                
            let mut pal_state = ListState::default();
            pal_state.select(Some(app.command_palette_selected));
            f.render_stateful_widget(palette, area, &mut pal_state);
        }
        Ok(())
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}
