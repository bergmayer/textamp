//! Terminal setup and cleanup.

use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

/// Setup the terminal for TUI mode.
pub fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to normal mode.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    // Disable mouse capture (may already be disabled by event loop shutdown)
    // and leave alternate screen BEFORE disabling raw mode
    // This ensures escape sequences are properly processed
    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);

    // Drain any remaining terminal input to prevent escape codes from being echoed
    use crossterm::event;
    while event::poll(std::time::Duration::from_millis(10)).unwrap_or(false) {
        let _ = event::read();
    }

    // Leave alternate screen and show cursor
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;

    // Flush to ensure all escape sequences are sent before disabling raw mode
    std::io::Write::flush(terminal.backend_mut())?;

    // Now disable raw mode
    disable_raw_mode()?;

    Ok(())
}
