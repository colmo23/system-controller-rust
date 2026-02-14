use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent};
use std::time::Duration;

pub enum AppEvent {
    Key(KeyEvent),
    None,
}

pub fn poll_event(timeout_ms: u64) -> Result<AppEvent> {
    if event::poll(Duration::from_millis(timeout_ms))? {
        if let Event::Key(key) = event::read()? {
            // Ignore key release events on some terminals
            if key.kind == crossterm::event::KeyEventKind::Press {
                return Ok(AppEvent::Key(key));
            }
        }
    }
    Ok(AppEvent::None)
}

