//! Native menu bar construction (muda).
//!
//! On macOS this is the global menu bar; on Windows / Linux it attaches
//! to the window frame as a classic dropdown. Only compiled when the
//! `native-menus` feature is enabled — Linux environments without GTK3
//! fall back to an in-window software menu (TODO: Step 10).
//!
//! The menu is built once in `init()` and the returned `Menu` is kept
//! alive for the lifetime of the application. Item clicks arrive on
//! muda's global `MenuEvent` channel, which the app's subscription
//! forwards into `GuiMessage::Action(...)`.

#![cfg(feature = "native-menus")]

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    Menu, MenuItem, PredefinedMenuItem, Submenu,
};

use crate::app::Action;
use crate::app::action::{NavigationAction, PlaybackAction, QueueAction, SettingsAction, SystemAction};
use crate::app::state::{BrowseCategory, View};

use super::message::GuiMessage;

/// Stable numeric IDs for menu items. Used by the action resolver.
pub mod ids {
    // File
    pub const FILE_SIGN_OUT:      u32 = 1001;
    pub const FILE_SWITCH_LIBRARY: u32 = 1002;
    pub const FILE_QUIT:          u32 = 1003;

    // View
    pub const VIEW_LIBRARY:     u32 = 2001;
    pub const VIEW_PLAYLISTS:   u32 = 2002;
    pub const VIEW_GENRES:      u32 = 2003;
    pub const VIEW_FOLDERS:     u32 = 2004;
    pub const VIEW_QUEUE:       u32 = 2005;
    pub const VIEW_NOW_PLAYING: u32 = 2006;

    // Playback
    pub const PB_PLAY_PAUSE: u32 = 3001;
    pub const PB_PREV:       u32 = 3002;
    pub const PB_NEXT:       u32 = 3003;
    pub const PB_SEEK_BACK:  u32 = 3004;
    pub const PB_SEEK_FWD:   u32 = 3005;
    pub const PB_VOL_UP:     u32 = 3006;
    pub const PB_VOL_DOWN:   u32 = 3007;

    // Queue
    pub const Q_ENQUEUE:     u32 = 4001;
    pub const Q_ENQUEUE_NEXT: u32 = 4002;
    pub const Q_SAVE:        u32 = 4003;
    pub const Q_CLEAR:       u32 = 4004;

    // Tools
    pub const TOOLS_REFRESH:  u32 = 5001;
    pub const TOOLS_SETTINGS: u32 = 5002;

    // Help
    pub const HELP_SHORTCUTS: u32 = 6001;
    pub const HELP_ABOUT:     u32 = 6002;
}

/// Take the process-wide muda menu-event receiver. Returns `Some` exactly
/// once; subsequent calls return `None`. Iced's subscription invokes this
/// during its first `subscription()` evaluation and streams menu events
/// into the app.
pub fn take_event_receiver() -> Option<muda::MenuEventReceiver> {
    Some(muda::MenuEvent::receiver().clone())
}

/// Build and return the top-level menu bar. Keep the returned value alive
/// for the process lifetime — dropping it tears the menu bar down.
pub fn build() -> muda::Menu {
    let menu = Menu::new();

    // ── File ───────────────────────────────────────────────────────────
    let file = Submenu::new("&File", true);
    file.append_items(&[
        &MenuItem::with_id(ids::FILE_SWITCH_LIBRARY, "Switch Library\u{2026}", true, accel(Modifiers::empty(), Code::F3)),
        &MenuItem::with_id(ids::FILE_SIGN_OUT,      "Sign Out",                   true, None),
        &PredefinedMenuItem::separator(),
        // Quit accelerator: Cmd+Q on macOS, Alt+F4 on Windows, Ctrl+Q
        // on Linux. The shared key handler accepts Cmd+W on macOS too
        // (single-window app convention) but muda only displays one
        // accelerator per item; the menu shows the most idiomatic.
        &MenuItem::with_id(ids::FILE_QUIT, "Quit", true, quit_accel()),
    ]).ok();
    menu.append(&file).ok();

    // ── View ───────────────────────────────────────────────────────────
    let view = Submenu::new("&View", true);
    view.append_items(&[
        &MenuItem::with_id(ids::VIEW_LIBRARY,   "Library",     true, accel(cmd_or_ctrl(), Code::KeyL)),
        &MenuItem::with_id(ids::VIEW_PLAYLISTS, "Playlists",   true, accel(cmd_or_ctrl(), Code::KeyP)),
        &MenuItem::with_id(ids::VIEW_GENRES,    "Genres",      true, accel(cmd_or_ctrl(), Code::KeyG)),
        &MenuItem::with_id(ids::VIEW_FOLDERS,   "Folders",     true, accel(cmd_or_ctrl(), Code::KeyO)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_QUEUE,        "Queue",        true, accel(cmd_or_ctrl(), Code::KeyU)),
        &MenuItem::with_id(ids::VIEW_NOW_PLAYING,  "Now Playing",  true, accel(cmd_or_ctrl(), Code::KeyN)),
    ]).ok();
    menu.append(&view).ok();

    // ── Playback ───────────────────────────────────────────────────────
    let playback = Submenu::new("&Playback", true);
    playback.append_items(&[
        &MenuItem::with_id(ids::PB_PLAY_PAUSE, "Play / Pause", true, accel(Modifiers::empty(), Code::Space)),
        &MenuItem::with_id(ids::PB_PREV,       "Previous Track", true, None),
        &MenuItem::with_id(ids::PB_NEXT,       "Next Track",     true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::PB_SEEK_BACK, "Seek Back 10s",    true, accel(Modifiers::SHIFT, Code::ArrowLeft)),
        &MenuItem::with_id(ids::PB_SEEK_FWD,  "Seek Forward 10s", true, accel(Modifiers::SHIFT, Code::ArrowRight)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::PB_VOL_UP,   "Volume Up",   true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::ArrowUp)),
        &MenuItem::with_id(ids::PB_VOL_DOWN, "Volume Down", true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::ArrowDown)),
    ]).ok();
    menu.append(&playback).ok();

    // ── Queue ──────────────────────────────────────────────────────────
    let queue = Submenu::new("&Queue", true);
    queue.append_items(&[
        &MenuItem::with_id(ids::Q_ENQUEUE,      "Add to end of queue", true, accel(cmd_or_ctrl(), Code::KeyE)),
        &MenuItem::with_id(ids::Q_ENQUEUE_NEXT, "Play next in queue",  true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::KeyE)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::Q_SAVE,  "Save queue as playlist\u{2026}", true, accel(cmd_or_ctrl(), Code::KeyS)),
        &MenuItem::with_id(ids::Q_CLEAR, "Clear Queue",              true, accel(cmd_or_ctrl(), Code::KeyX)),
    ]).ok();
    menu.append(&queue).ok();

    // ── Tools ──────────────────────────────────────────────────────────
    let tools = Submenu::new("&Tools", true);
    tools.append_items(&[
        &MenuItem::with_id(ids::TOOLS_REFRESH,  "Refresh",  true, accel(Modifiers::empty(), Code::F5)),
        &MenuItem::with_id(ids::TOOLS_SETTINGS, "Settings\u{2026}", true, accel(Modifiers::empty(), Code::F2)),
    ]).ok();
    menu.append(&tools).ok();

    // ── Help ───────────────────────────────────────────────────────────
    let help = Submenu::new("&Help", true);
    help.append_items(&[
        &MenuItem::with_id(ids::HELP_SHORTCUTS, "Keyboard Shortcuts", true, accel(Modifiers::empty(), Code::F1)),
        &MenuItem::with_id(ids::HELP_ABOUT,     "About Textamp",      true, None),
    ]).ok();
    menu.append(&help).ok();

    menu
}

/// Translate a clicked menu ID to the matching `GuiMessage`.
///
/// Most items resolve to a concrete `Action` that can be dispatched
/// directly. A few need the TUI's context-sensitive key handler to pick
/// the right action from the current `AppState` (e.g. F5 refresh, which
/// fans out to Artists / Albums / Queue / etc. depending on the active
/// view); those route through `GuiMessage::MenuKeyClick(KeyEvent)` so
/// the single source of truth remains `handlers::key_input::handle_key`.
///
/// `HELP_ABOUT` is not an `Action` — it toggles a GUI-local popup, so it
/// resolves to `GuiMessage::ShowAbout` directly.
pub fn menu_event_for_id(id: &str) -> Option<GuiMessage> {
    let id: u32 = id.parse().ok()?;
    let msg = match id {
        // ── Direct actions ─────────────────────────────────────────────
        ids::FILE_QUIT => GuiMessage::Action(Action::System(SystemAction::Quit)),
        ids::FILE_SIGN_OUT => GuiMessage::Action(Action::Settings(SettingsAction::Logout)),
        ids::VIEW_LIBRARY => GuiMessage::Action(Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Library))),
        ids::VIEW_PLAYLISTS => GuiMessage::Action(Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Playlists))),
        ids::VIEW_GENRES => GuiMessage::Action(Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Genres))),
        ids::VIEW_FOLDERS => GuiMessage::Action(Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Folders))),
        ids::VIEW_QUEUE => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Queue))),
        ids::VIEW_NOW_PLAYING => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::NowPlaying))),
        ids::PB_PLAY_PAUSE => GuiMessage::Action(Action::Playback(PlaybackAction::TogglePlayPause)),
        ids::PB_PREV => GuiMessage::Action(Action::Playback(PlaybackAction::Previous)),
        ids::PB_NEXT => GuiMessage::Action(Action::Playback(PlaybackAction::Next)),
        ids::PB_SEEK_BACK => GuiMessage::Action(Action::Playback(PlaybackAction::SeekRelative(-10_000))),
        ids::PB_SEEK_FWD => GuiMessage::Action(Action::Playback(PlaybackAction::SeekRelative(10_000))),
        ids::PB_VOL_UP => GuiMessage::Action(Action::Playback(PlaybackAction::VolumeUp)),
        ids::PB_VOL_DOWN => GuiMessage::Action(Action::Playback(PlaybackAction::VolumeDown)),
        ids::Q_ENQUEUE => GuiMessage::Action(Action::Queue(QueueAction::EnqueueSelection)),
        ids::Q_ENQUEUE_NEXT => GuiMessage::Action(Action::Queue(QueueAction::EnqueueSelectionNext)),
        ids::Q_SAVE => GuiMessage::Action(Action::Queue(QueueAction::PromptSavePlaylist)),
        ids::Q_CLEAR => GuiMessage::Action(Action::Queue(QueueAction::ClearQueue)),
        ids::TOOLS_SETTINGS => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Settings))),
        ids::HELP_SHORTCUTS => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Help))),
        ids::HELP_ABOUT => GuiMessage::ShowAbout,

        // ── Context-sensitive: delegate to key_input ──────────────────
        ids::FILE_SWITCH_LIBRARY => GuiMessage::MenuKeyClick(function_key(3)),
        ids::TOOLS_REFRESH => GuiMessage::MenuKeyClick(function_key(5)),

        _ => return None,
    };
    Some(msg)
}

/// Build a crossterm `KeyEvent` for a function key `Fn`. Used when a menu
/// item needs to trigger the same behaviour as pressing the matching F-key
/// in the TUI.
fn function_key(n: u8) -> KeyEvent {
    KeyEvent {
        code: KeyCode::F(n),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

// Platform helpers -----------------------------------------------------------

#[inline]
fn accel(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(mods), code))
}

/// `Cmd` on macOS, `Ctrl` elsewhere. muda already does this internally for
/// `Modifiers::META` on macOS vs `Modifiers::CONTROL` elsewhere — we use
/// `CONTROL` and let accelerator mapping do the work.
#[inline]
fn cmd_or_ctrl() -> Modifiers {
    #[cfg(target_os = "macos")] { Modifiers::META }
    #[cfg(not(target_os = "macos"))] { Modifiers::CONTROL }
}

/// Platform-conventional Quit accelerator:
///   - macOS:   Cmd+Q
///   - Windows: Alt+F4
///   - Linux:   Ctrl+Q
fn quit_accel() -> Option<Accelerator> {
    #[cfg(target_os = "macos")]
    { accel(Modifiers::META, Code::KeyQ) }
    #[cfg(target_os = "windows")]
    { accel(Modifiers::ALT, Code::F4) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { accel(Modifiers::CONTROL, Code::KeyQ) }
}
