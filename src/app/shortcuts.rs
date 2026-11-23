//! Application navigation bindings shared by terminal and windowed adapters.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::PaletteCommandKind;

/// Application destinations available through global navigation bindings.
#[derive(Clone, Copy)]
pub enum NavigationCommand {
    /// Open the library workspace.
    Library,
    /// Toggle or focus the play queue according to the presentation.
    Queue,
    /// Open Now Playing.
    NowPlaying,
    /// Open Settings.
    Settings,
    /// Browse a random album.
    RandomAlbum,
    Radio,
    Journey,
    AlbumMix,
    LibraryMix,

    /// Search the current music selection in the Apple Music app.
    SearchAppleMusic,
}

impl NavigationCommand {
    /// The shared command executed by every frontend.
    pub fn command(self) -> PaletteCommandKind {
        match self {
            Self::Library => PaletteCommandKind::GotoLibrary,
            Self::Queue => PaletteCommandKind::GotoQueue,
            Self::NowPlaying => PaletteCommandKind::GotoNowPlaying,
            Self::Settings => PaletteCommandKind::OpenSettings,
            Self::SearchAppleMusic => PaletteCommandKind::SearchAppleMusic,
            Self::Radio => PaletteCommandKind::OpenRadioBrowser,
            Self::Journey => PaletteCommandKind::SonicAdventure,
            Self::AlbumMix => PaletteCommandKind::PlayRandomAlbums,
            Self::LibraryMix => PaletteCommandKind::LibraryMix,
            Self::RandomAlbum => PaletteCommandKind::GoToRandomAlbum,
        }
    }
}

/// A navigation command and its terminal and native keyboard bindings.
pub struct NavigationShortcut {
    /// Normalized terminal key.
    pub key: KeyEvent,
    /// Equivalent native Command and Control bindings.
    pub windowed: &'static [&'static str],
    /// Command dispatched by either adapter.
    pub command: NavigationCommand,
}

/// Application navigation bindings that remain available inside panels and forms.
pub const NAVIGATION: &[NavigationShortcut] = &[
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL.union(KeyModifiers::ALT),
        ),
        windowed: &["cmd-alt-r", "ctrl-alt-r"],
        command: NavigationCommand::Radio,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL.union(KeyModifiers::ALT),
        ),
        windowed: &["cmd-alt-j", "ctrl-alt-j"],
        command: NavigationCommand::Journey,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL.union(KeyModifiers::ALT),
        ),
        windowed: &["cmd-alt-a", "ctrl-alt-a"],
        command: NavigationCommand::AlbumMix,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL.union(KeyModifiers::ALT),
        ),
        windowed: &["cmd-alt-l", "ctrl-alt-l"],
        command: NavigationCommand::LibraryMix,
    },
    NavigationShortcut {
        key: KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL),
        windowed: &["cmd-1", "ctrl-1"],
        command: NavigationCommand::Library,
    },
    NavigationShortcut {
        key: KeyEvent::new(KeyCode::Char('2'), KeyModifiers::CONTROL),
        windowed: &["cmd-2", "ctrl-2"],
        command: NavigationCommand::Queue,
    },
    NavigationShortcut {
        key: KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        windowed: &["cmd-u", "ctrl-u"],
        command: NavigationCommand::Queue,
    },
    NavigationShortcut {
        key: KeyEvent::new(KeyCode::Char('3'), KeyModifiers::CONTROL),
        windowed: &["cmd-3", "ctrl-3"],
        command: NavigationCommand::NowPlaying,
    },
    NavigationShortcut {
        key: KeyEvent::new(KeyCode::Char(','), KeyModifiers::CONTROL),
        windowed: &["cmd-,", "ctrl-,"],
        command: NavigationCommand::Settings,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
        ),
        windowed: &["cmd-shift-r", "ctrl-shift-r"],
        command: NavigationCommand::RandomAlbum,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
        ),
        windowed: &["cmd-shift-a", "ctrl-shift-a"],
        command: NavigationCommand::SearchAppleMusic,
    },
    NavigationShortcut {
        key: KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::SUPER.union(KeyModifiers::SHIFT),
        ),
        windowed: &[],
        command: NavigationCommand::SearchAppleMusic,
    },
];

/// Resolve a normalized navigation key without interpreting text or repeat state.
pub fn navigation(key: KeyEvent) -> Option<PaletteCommandKind> {
    let code = match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::SHIFT) => {
            KeyCode::Char(ch.to_ascii_lowercase())
        }
        code => code,
    };
    NAVIGATION
        .iter()
        .find(|binding| binding.key.code == code && binding.key.modifiers == key.modifiers)
        .map(|binding| binding.command.command())
}
