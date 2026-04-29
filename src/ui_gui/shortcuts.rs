//! Translate Iced keyboard events into `Action`s.
//!
//! Mirrors the mapping in `crate::app::handlers::key_input::handle_key` so
//! the GUI fires the same Actions as the TUI for the same key combinations.
//!
//! Iced reports keys as `iced::keyboard::Key` — we convert them into
//! `crossterm::event::KeyEvent` and hand them to the existing key_input
//! layer. The mapping lives inside the TUI-neutral `key_input` module
//! precisely so both front-ends can call it.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use iced::keyboard::{self, key::Named};

/// Convert an Iced key press into a `crossterm::KeyEvent` that the shared
/// `handlers::key_input::handle_key` understands.
///
/// Returns `None` when the press has no meaning (non-character dead keys,
/// system keys we don't handle).
pub fn to_crossterm_key_event(
    key: keyboard::Key,
    modifiers: keyboard::Modifiers,
) -> Option<KeyEvent> {
    let code = iced_to_crossterm_code(&key)?;
    Some(KeyEvent::new_with_kind_and_state(
        code,
        iced_to_crossterm_mods(modifiers),
        KeyEventKind::Press,
        KeyEventState::NONE,
    ))
}

fn iced_to_crossterm_code(key: &keyboard::Key) -> Option<KeyCode> {
    use keyboard::Key as K;
    Some(match key {
        K::Named(Named::ArrowUp)    => KeyCode::Up,
        K::Named(Named::ArrowDown)  => KeyCode::Down,
        K::Named(Named::ArrowLeft)  => KeyCode::Left,
        K::Named(Named::ArrowRight) => KeyCode::Right,
        K::Named(Named::PageUp)     => KeyCode::PageUp,
        K::Named(Named::PageDown)   => KeyCode::PageDown,
        K::Named(Named::Home)       => KeyCode::Home,
        K::Named(Named::End)        => KeyCode::End,
        K::Named(Named::Enter)      => KeyCode::Enter,
        K::Named(Named::Escape)     => KeyCode::Esc,
        K::Named(Named::Backspace)  => KeyCode::Backspace,
        K::Named(Named::Delete)     => KeyCode::Delete,
        K::Named(Named::Tab)        => KeyCode::Tab,
        K::Named(Named::Space)      => KeyCode::Char(' '),
        K::Named(Named::F1)         => KeyCode::F(1),
        K::Named(Named::F2)         => KeyCode::F(2),
        K::Named(Named::F3)         => KeyCode::F(3),
        K::Named(Named::F4)         => KeyCode::F(4),
        K::Named(Named::F5)         => KeyCode::F(5),
        K::Named(Named::F6)         => KeyCode::F(6),
        K::Named(Named::F7)         => KeyCode::F(7),
        K::Named(Named::F8)         => KeyCode::F(8),
        K::Named(Named::F9)         => KeyCode::F(9),
        K::Named(Named::F10)        => KeyCode::F(10),
        K::Named(Named::F11)        => KeyCode::F(11),
        K::Named(Named::F12)        => KeyCode::F(12),
        K::Character(s) => {
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() { return None; }
            KeyCode::Char(c)
        }
        _ => return None,
    })
}

fn iced_to_crossterm_mods(m: keyboard::Modifiers) -> KeyModifiers {
    let mut out = KeyModifiers::NONE;
    if m.shift() { out |= KeyModifiers::SHIFT; }
    if m.alt()   { out |= KeyModifiers::ALT; }
    // Mac GUI: Cmd is the platform shortcut modifier and Ctrl must
    // NOT trigger CONTROL-bound shortcuts. We map Cmd onto CONTROL
    // (so the shared `key_input::handle_key` fires Cmd+F / Cmd+L /
    // Cmd+P / Cmd+W / Cmd+Q just like the TUI's Ctrl+… equivalents)
    // and route a real Ctrl press to SUPER instead — no shared
    // handler matches SUPER for those bindings, so Ctrl+anything is
    // effectively inert in the Mac GUI. This lines up with macOS
    // platform conventions where Ctrl is reserved for emacs-style
    // text edit shortcuts.
    #[cfg(target_os = "macos")]
    {
        if m.control() { out |= KeyModifiers::SUPER; }
        if m.logo()    { out |= KeyModifiers::CONTROL; }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if m.control() { out |= KeyModifiers::CONTROL; }
        if m.logo()    { out |= KeyModifiers::SUPER; }
    }
    out
}

