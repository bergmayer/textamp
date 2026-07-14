//! Terminal setup and cleanup.

use std::io::{self, Stdout, Write};
use std::ops::{Deref, DerefMut};
use std::panic::{catch_unwind, AssertUnwindSafe};

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

pub type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

thread_local! {
    /// Depth rather than a bool so nested guarded decoder operations remain safe.
    static EXPECTED_PANIC_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Catch a known third-party panic without temporarily replacing the global
/// panic hook. The permanent terminal-safety hook consults this thread-local
/// guard and stays active for unrelated threads.
pub(crate) fn catch_expected_panic<F, R>(operation: F) -> std::thread::Result<R>
where
    F: FnOnce() -> R,
{
    EXPECTED_PANIC_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
    let result = catch_unwind(AssertUnwindSafe(operation));
    EXPECTED_PANIC_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    result
}

fn expected_panic_on_this_thread() -> bool {
    EXPECTED_PANIC_DEPTH.with(|depth| depth.get() > 0)
}

/// Owns the configured terminal and restores it when dropped.
///
/// Cleanup is deliberately idempotent and best-effort: a failure writing one
/// escape sequence must never prevent raw mode from being disabled.
pub struct TerminalSession {
    terminal: AppTerminal,
    restored: bool,
}

impl Deref for TerminalSession {
    type Target = AppTerminal;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for TerminalSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl TerminalSession {
    /// Restore the terminal now. Subsequent calls, including `Drop`, are no-ops.
    pub fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        restore_backend(self.terminal.backend_mut())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn remember_first(first: &mut Option<io::Error>, result: io::Result<()>) {
    if first.is_none() {
        if let Err(error) = result {
            *first = Some(error);
        }
    }
}

/// Restore terminal state using any writable terminal output.
///
/// Each operation is attempted independently. In particular,
/// `disable_raw_mode` always runs, even if writing an escape sequence fails.
fn restore_writer<W: Write>(writer: &mut W) -> io::Result<()> {
    let mut first_error = None;

    remember_first(
        &mut first_error,
        execute!(writer, PopKeyboardEnhancementFlags),
    );
    remember_first(&mut first_error, execute!(writer, DisableMouseCapture));
    remember_first(&mut first_error, execute!(writer, LeaveAlternateScreen));
    remember_first(&mut first_error, execute!(writer, cursor::Show));
    remember_first(&mut first_error, writer.flush());
    remember_first(&mut first_error, disable_raw_mode());

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn restore_backend(backend: &mut CrosstermBackend<Stdout>) -> io::Result<()> {
    restore_writer(backend)
}

fn restore_stdio_best_effort() {
    let mut stdout = io::stdout();
    let _ = restore_writer(&mut stdout);
}

/// Setup the terminal for TUI mode.
///
/// Every setup failure rolls back all state already enabled. The returned
/// session provides a final RAII guard for clean exits, errors, and unwinding.
pub fn setup_terminal() -> io::Result<TerminalSession> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();

    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        restore_stdio_best_effort();
        return Err(error);
    }
    if let Err(error) = execute!(stdout, EnableMouseCapture) {
        restore_stdio_best_effort();
        return Err(error);
    }

    // Unsupported terminals harmlessly ignore this protocol extension.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );

    let backend = CrosstermBackend::new(stdout);
    let terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            restore_stdio_best_effort();
            return Err(error);
        }
    };

    Ok(TerminalSession {
        terminal,
        restored: false,
    })
}

/// Install a panic hook that restores the terminal before the default hook
/// prints the panic message.
///
/// This is a single process-wide hook. Library code must not replace it
/// temporarily; doing so would race panics on other threads.
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if !expected_panic_on_this_thread() {
            restore_stdio_best_effort();
            original_hook(panic_info);
        }
    }));
}

/// Restore a terminal session to normal mode.
pub fn restore_terminal(session: &mut TerminalSession) -> io::Result<()> {
    session.restore()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
    }

    #[test]
    fn cleanup_reports_output_failure_without_panicking() {
        let mut writer = FailingWriter;
        assert!(restore_writer(&mut writer).is_err());
    }
}
