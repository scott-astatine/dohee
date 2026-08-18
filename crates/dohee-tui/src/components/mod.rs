use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};
use crate::action::Action;
use crate::app::TuiApp;

pub mod header;
pub mod transcript;
pub mod composer;
pub mod status_bar;
pub mod popups;

pub trait Component {
    /// Keyboard input handler mapping hardware keys to actions.
    fn handle_key_event(&mut self, _key: KeyEvent, _app: &mut TuiApp) -> Result<Option<Action>> {
        Ok(None)
    }

    /// Update component state after receiving a dispatched Action.
    fn update(&mut self, _action: &Action, _app: &mut TuiApp) -> Result<Option<Action>> {
        Ok(None)
    }

    /// Render layout graphics to target screen frame.
    fn draw(&mut self, f: &mut Frame, rect: Rect, app: &mut TuiApp) -> Result<()>;
}
