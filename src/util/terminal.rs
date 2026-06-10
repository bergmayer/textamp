//! Terminal setup and cleanup.

use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

/// Setup the terminal for TUI mode.
///
/// Pushes the kitty keyboard-enhancement flags so we can disambiguate
/// modified keys that legacy escape codes can't (Shift+Space, Ctrl+i
/// vs Tab, etc.). Falls through silently when the host terminal
/// doesn't support the protocol — the push just becomes a no-op.
pub fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Install a panic hook that restores the terminal before the default
/// hook prints the panic message. Without this, a panic anywhere in the
/// event loop leaves the terminal in raw mode on the alternate screen:
/// the shell is unusable and the panic message is invisible.
///
/// Must be installed before `setup_terminal()`. The restore is
/// best-effort and idempotent, so the normal `restore_terminal()` on
/// the clean exit path is unaffected.
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            LeaveAlternateScreen,
            cursor::Show
        );
        let _ = disable_raw_mode();
        original_hook(panic_info);
    }));
}

/// Restore the terminal to normal mode.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    // Pop the keyboard-enhancement push from setup_terminal. Safe to
    // call even when the push was a no-op.
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);

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
