//! Native menu bar construction (muda).
//!
//! On macOS this is the global menu bar (top of the screen). On
//! Windows / Linux a parallel in-window menu in
//! `widgets::menu_bar` is used; this file is only compiled when the
//! `native-menus` feature is enabled.
//!
//! Layout mirrors the in-window Windows menu (`widgets::menu_bar`)
//! exactly so the two front-ends stay in lockstep, including all
//! accelerators (with `Cmd` substituted for `Ctrl` on macOS).
//!
//! The menu is built from `App::update`'s first tick on the main
//! thread (winit's launch hook installs its own bare-bones menu in
//! `applicationDidFinishLaunching:`, which would otherwise overwrite
//! ours) and the returned `Menu` is leaked so Drop doesn't tear down
//! the NSMenu NSApplication retains.
//!
//! Item clicks arrive on muda's global `MenuEvent` channel, which the
//! app's subscription forwards into `GuiMessage::Action(...)`.

#![cfg(feature = "native-menus")]

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    Menu, MenuItem, PredefinedMenuItem, Submenu,
};

use crate::app::Action;
use crate::app::action::{
    NavigationAction, PlaybackAction, QueueAction, SearchAction, SettingsAction, SystemAction,
};
use crate::app::state::{BrowseCategory, ColumnSortMode, View};

use super::message::GuiMessage;

/// Stable numeric IDs for menu items. Used by the action resolver.
pub mod ids {
    // ── File ───────────────────────────────────────────────────────────
    pub const FILE_ABOUT:          u32 = 1000;
    pub const FILE_SETTINGS:       u32 = 1001;
    pub const FILE_SWITCH_LIBRARY: u32 = 1002;
    pub const FILE_SIGN_OUT:       u32 = 1003;
    pub const FILE_QUIT:           u32 = 1004;

    // ── View ───────────────────────────────────────────────────────────
    pub const VIEW_BROWSE:           u32 = 2000;
    pub const VIEW_QUEUE:            u32 = 2001;
    pub const VIEW_NOW_PLAYING:      u32 = 2002;
    pub const VIEW_LIBRARY:          u32 = 2003;
    pub const VIEW_PLAYLISTS:        u32 = 2004;
    pub const VIEW_GENRES:           u32 = 2005;
    pub const VIEW_FOLDERS:          u32 = 2006;
    pub const VIEW_SIMILAR:          u32 = 2007;
    pub const VIEW_RELATED:          u32 = 2008;
    pub const VIEW_OPEN_IN_LIBRARY:  u32 = 2009;
    pub const VIEW_ARTIST_BIO:       u32 = 2010;
    pub const VIEW_SORT_DEFAULT:     u32 = 2011;
    pub const VIEW_SORT_ARTIST:      u32 = 2012;
    pub const VIEW_SORT_ALBUM:       u32 = 2013;
    pub const VIEW_SORT_TITLE:       u32 = 2014;
    pub const VIEW_SORT_DURATION:    u32 = 2015;
    pub const VIEW_SORT_SHUFFLE:     u32 = 2016;
    pub const VIEW_REVERSE_SORT:     u32 = 2017;
    pub const VIEW_GROUP_BY_ALBUM:   u32 = 2018;
    pub const VIEW_TOGGLE_COVER_ART: u32 = 2019;
    pub const VIEW_SCROLLING_LAYOUT: u32 = 2020;
    pub const VIEW_TALL_MODE:        u32 = 2021;

    // ── Playback ───────────────────────────────────────────────────────
    pub const PB_PLAY_PAUSE: u32 = 3000;
    pub const PB_STOP:       u32 = 3001;
    pub const PB_PREV:       u32 = 3002;
    pub const PB_NEXT:       u32 = 3003;
    pub const PB_SEEK_BACK:  u32 = 3004;
    pub const PB_SEEK_FWD:   u32 = 3005;
    pub const PB_VOL_UP:     u32 = 3006;
    pub const PB_VOL_DOWN:   u32 = 3007;
    pub const PB_MUTE:       u32 = 3008;

    // ── Queue ──────────────────────────────────────────────────────────
    pub const Q_ENQUEUE:      u32 = 4000;
    pub const Q_ENQUEUE_NEXT: u32 = 4001;
    pub const Q_SAVE:         u32 = 4002;
    pub const Q_CLEAR:        u32 = 4003;
    pub const Q_SHUFFLE:      u32 = 4004;

    // ── Queue → DJ Modes ──────────────────────────────────────────────
    pub const Q_DJ_STRETCH:  u32 = 4100;
    pub const Q_DJ_GEMINI:   u32 = 4101;
    pub const Q_DJ_FREEZE:   u32 = 4102;
    pub const Q_DJ_TWOFER:   u32 = 4103;
    pub const Q_DJ_CONTEMPO: u32 = 4104;
    pub const Q_DJ_GROUPIE:  u32 = 4105;

    // ── Queue → Remix tools ───────────────────────────────────────────
    pub const Q_REMIX_GEMINI:        u32 = 4200;
    pub const Q_REMIX_TWOFER:        u32 = 4201;
    pub const Q_REMIX_STRETCH:       u32 = 4202;
    pub const Q_REMIX_DOPPELGANGER:  u32 = 4203;
    pub const Q_REMIX_SHUFFLE:       u32 = 4204;
    pub const Q_REMIX_UNDO_SHUFFLE:  u32 = 4205;

    // ── Radio (top-level menu) ────────────────────────────────────────
    pub const RADIO_ARTIST:       u32 = 7000;
    pub const RADIO_ADVENTURE:    u32 = 7001;
    pub const RADIO_STATIONS:     u32 = 7003;

    // ── Tools ──────────────────────────────────────────────────────────
    pub const TOOLS_SEARCH:           u32 = 5000;
    pub const TOOLS_ADVENTURE:        u32 = 5001;
    pub const TOOLS_ARTIST_RADIO:     u32 = 5002;
    pub const TOOLS_RANDOM_ALBUM:     u32 = 5003;
    pub const TOOLS_REFRESH:          u32 = 5004;
    pub const TOOLS_SEARCH_APPLE:     u32 = 5005;
    pub const TOOLS_SEARCH_SPOTIFY:   u32 = 5006;
    pub const TOOLS_SEARCH_YOUTUBE:   u32 = 5007;
    pub const TOOLS_PALETTE:          u32 = 5008;

    // ── Help ───────────────────────────────────────────────────────────
    pub const HELP_USER_GUIDE: u32 = 6000;
    pub const HELP_SHORTCUTS:  u32 = 6001;
}

/// Forwarded menu events — populated lazily by `install_event_forwarder`,
/// drained by the iced subscription. We use a tokio unbounded channel so
/// the receiver side is async-pollable; muda's own receiver is a
/// crossbeam channel whose `recv()` is synchronously blocking, which
/// behaves badly inside an iced/tokio subscription stream.
static MENU_FORWARDER: std::sync::OnceLock<
    std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<GuiMessage>>>,
> = std::sync::OnceLock::new();

/// Install muda's process-wide menu event handler. Runs from the first
/// `App::update` tick (same place as `init_for_nsapp`) and forwards each
/// event into a tokio unbounded channel. Idempotent — only the first
/// caller registers the handler; subsequent calls are no-ops.
pub fn install_event_forwarder() {
    if MENU_FORWARDER.get().is_some() {
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<GuiMessage>();
    let _ = MENU_FORWARDER.set(std::sync::Mutex::new(Some(rx)));
    muda::MenuEvent::set_event_handler(Some(move |ev: muda::MenuEvent| {
        if let Some(msg) = menu_event_for_id(ev.id.0.as_str()) {
            let _ = tx.send(msg);
        }
    }));
    tracing::info!("muda MenuEvent handler installed");
}

/// Take the forwarder's receiver. Returns `Some` exactly once; later
/// calls return `None` so re-evaluations of `subscription()` don't
/// steal the live receiver from the running task.
pub fn take_forwarder_receiver() -> Option<tokio::sync::mpsc::UnboundedReceiver<GuiMessage>> {
    MENU_FORWARDER.get()?.lock().ok()?.take()
}

/// Build the top-level menu. Layout mirrors `widgets::menu_bar::items_for`.
pub fn build() -> muda::Menu {
    let menu = Menu::new();

    // ── File ───────────────────────────────────────────────────────────
    // On macOS, the leftmost submenu's title is replaced by the app
    // name in bold — putting About / Settings / Quit here matches both
    // Windows convention and Mac's app-menu convention.
    let file = Submenu::new("&File", true);
    file.append_items(&[
        &MenuItem::with_id(ids::FILE_ABOUT,          "About Textamp",            true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::FILE_SETTINGS,       "Settings\u{2026}",         true, accel(Modifiers::empty(), Code::F2)),
        &MenuItem::with_id(ids::FILE_SWITCH_LIBRARY, "Switch Library\u{2026}",   true, accel(Modifiers::empty(), Code::F3)),
        &MenuItem::with_id(ids::FILE_SIGN_OUT,       "Sign Out",                 true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::FILE_QUIT,           "Quit",                     true, quit_accel()),
    ]).ok();
    menu.append(&file).ok();

    // ── View ───────────────────────────────────────────────────────────
    let view = Submenu::new("&View", true);
    view.append_items(&[
        &MenuItem::with_id(ids::VIEW_BROWSE,         "Browse",                       true, None),
        &MenuItem::with_id(ids::VIEW_QUEUE,          "Queue",                        true, accel(cmd_or_ctrl(), Code::KeyU)),
        &MenuItem::with_id(ids::VIEW_NOW_PLAYING,    "Now Playing",                  true, accel(cmd_or_ctrl(), Code::KeyN)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_LIBRARY,        "Library",                      true, accel(cmd_or_ctrl(), Code::KeyL)),
        // Playlists used to live on Cmd+P; that's now the command
        // palette shortcut (matching VS Code / many other apps).
        // Playlists is still reachable from the leftmost browse
        // column and from this menu — just without an accelerator.
        &MenuItem::with_id(ids::VIEW_PLAYLISTS,      "Playlists",                    true, None),
        &MenuItem::with_id(ids::VIEW_GENRES,         "Genres",                       true, accel(cmd_or_ctrl(), Code::KeyG)),
        &MenuItem::with_id(ids::VIEW_FOLDERS,        "Folders",                      true, accel(cmd_or_ctrl(), Code::KeyO)),
        &PredefinedMenuItem::separator(),
        // The Similar entry is context-aware (the shared key handler
        // picks track / album / artist similarity from whatever's
        // selected — falling back to the currently-playing track).
        // The label reads "Show Similar…" so that's clear; users
        // looking specifically for tracks-like-this or albums-like-
        // this also have the right-click context menus on every
        // track / album row in Browse and the queue.
        &MenuItem::with_id(ids::VIEW_SIMILAR,         "Show Similar\u{2026}",        true, accel(cmd_or_ctrl(), Code::KeyM)),
        &MenuItem::with_id(ids::VIEW_RELATED,         "Related Artists\u{2026}",     true, accel(cmd_or_ctrl(), Code::KeyR)),
        &MenuItem::with_id(ids::VIEW_OPEN_IN_LIBRARY, "Open in Library",             true, accel(cmd_or_ctrl(), Code::KeyJ)),
        &MenuItem::with_id(ids::VIEW_ARTIST_BIO,      "Artist Bio",                  true, accel(Modifiers::empty(), Code::F4)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_SORT_DEFAULT,    "Sort: Default",               true, None),
        &MenuItem::with_id(ids::VIEW_SORT_ARTIST,     "Sort: By Artist",             true, None),
        &MenuItem::with_id(ids::VIEW_SORT_ALBUM,      "Sort: By Album",              true, None),
        &MenuItem::with_id(ids::VIEW_SORT_TITLE,      "Sort: By Title",              true, None),
        &MenuItem::with_id(ids::VIEW_SORT_DURATION,   "Sort: By Duration",           true, None),
        &MenuItem::with_id(ids::VIEW_SORT_SHUFFLE,    "Sort: Shuffle",               true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_REVERSE_SORT,    "Reverse Sort Direction",      true, None),
        &MenuItem::with_id(ids::VIEW_GROUP_BY_ALBUM,  "Group by Album",              true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_TOGGLE_COVER_ART, "Toggle Cover Art",           true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::VIEW_SCROLLING_LAYOUT, "Toggle Scrolling Layout",    true, accel(Modifiers::empty(), Code::Backslash)),
        &MenuItem::with_id(ids::VIEW_TALL_MODE,        "Toggle Tall Mode",           true, accel(Modifiers::SHIFT, Code::Backslash)),
    ]).ok();
    menu.append(&view).ok();

    // ── Playback ───────────────────────────────────────────────────────
    let playback = Submenu::new("&Playback", true);
    playback.append_items(&[
        &MenuItem::with_id(ids::PB_PLAY_PAUSE, "Play / Pause",      true, accel(Modifiers::empty(), Code::Space)),
        &MenuItem::with_id(ids::PB_STOP,       "Stop",              true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::PB_PREV,       "Previous Track",    true, None),
        &MenuItem::with_id(ids::PB_NEXT,       "Next Track",        true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::PB_SEEK_BACK,  "Seek Back 10s",     true, accel(Modifiers::SHIFT, Code::ArrowLeft)),
        &MenuItem::with_id(ids::PB_SEEK_FWD,   "Seek Forward 10s",  true, accel(Modifiers::SHIFT, Code::ArrowRight)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::PB_VOL_UP,     "Volume Up",         true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::ArrowUp)),
        &MenuItem::with_id(ids::PB_VOL_DOWN,   "Volume Down",       true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::ArrowDown)),
        &MenuItem::with_id(ids::PB_MUTE,       "Mute / Unmute",     true, None),
    ]).ok();
    menu.append(&playback).ok();

    // ── Queue ──────────────────────────────────────────────────────────
    let queue = Submenu::new("&Queue", true);
    queue.append_items(&[
        &MenuItem::with_id(ids::Q_ENQUEUE,      "Add to end of queue",            true, accel(cmd_or_ctrl(), Code::KeyE)),
        &MenuItem::with_id(ids::Q_ENQUEUE_NEXT, "Play next in queue",             true, accel(cmd_or_ctrl() | Modifiers::SHIFT, Code::KeyE)),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::Q_SAVE,         "Save queue as playlist\u{2026}", true, accel(cmd_or_ctrl(), Code::KeyS)),
        &MenuItem::with_id(ids::Q_CLEAR,        "Clear Queue",                    true, accel(cmd_or_ctrl(), Code::KeyX)),
        &MenuItem::with_id(ids::Q_SHUFFLE,      "Shuffle",                        true, None),
        &PredefinedMenuItem::separator(),
        // ── DJ Modes ──
        &MenuItem::with_id(ids::Q_DJ_STRETCH,   "DJ Stretch",   true, None),
        &MenuItem::with_id(ids::Q_DJ_GEMINI,    "DJ Gemini",    true, None),
        &MenuItem::with_id(ids::Q_DJ_FREEZE,    "DJ Freeze",    true, None),
        &MenuItem::with_id(ids::Q_DJ_TWOFER,    "DJ Twofer",    true, None),
        &MenuItem::with_id(ids::Q_DJ_CONTEMPO,  "DJ Contempo",  true, None),
        &MenuItem::with_id(ids::Q_DJ_GROUPIE,   "DJ Groupie",   true, None),
        &PredefinedMenuItem::separator(),
        // ── Remix tools ──
        &MenuItem::with_id(ids::Q_REMIX_GEMINI,       "Remix: Gemini",        true, None),
        &MenuItem::with_id(ids::Q_REMIX_TWOFER,       "Remix: Twofer",        true, None),
        &MenuItem::with_id(ids::Q_REMIX_STRETCH,      "Remix: Stretch",       true, None),
        &MenuItem::with_id(ids::Q_REMIX_DOPPELGANGER, "Remix: Doppelganger",  true, None),
        &MenuItem::with_id(ids::Q_REMIX_SHUFFLE,      "Remix: Shuffle",       true, None),
        &MenuItem::with_id(ids::Q_REMIX_UNDO_SHUFFLE, "Remix: Undo Shuffle",  true, None),
    ]).ok();
    menu.append(&queue).ok();

    // ── Radio ──────────────────────────────────────────────────────────
    // Top-level menu separate from Queue: starting a radio is a
    // playback-source switch, not queue-management. The "Stations…"
    // entry opens the existing Stations popup which lists every Plex
    // station and drills into per-mood/style/decade categories —
    // that's the practical equivalent of a Stations submenu without
    // muda's submenu requiring static at-build-time content (Plex
    // stations are dynamic per library).
    let radio = Submenu::new("&Radio", true);
    radio.append_items(&[
        &MenuItem::with_id(ids::RADIO_ARTIST,    "Artist Radio\u{2026}",    true, None),
        &MenuItem::with_id(ids::RADIO_ADVENTURE, "Adventure\u{2026}",       true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::RADIO_STATIONS,  "Stations\u{2026}",        true, None),
    ]).ok();
    menu.append(&radio).ok();

    // ── Tools ──────────────────────────────────────────────────────────
    // Tools is for "everything else". Adventure / Artist Radio /
    // Stations live in Radio (they all spin up a streaming queue);
    // Random Album lives here because it's a one-off "play a random
    // album" command, not a radio source.
    let tools = Submenu::new("&Tools", true);
    tools.append_items(&[
        &MenuItem::with_id(ids::TOOLS_PALETTE,       "Command Palette\u{2026}", true,
            // Cmd+P (was Playlists). The shared `key_input::handle_key`
            // also opens the palette on `:` directly, so both paths
            // work on every platform — the menu accelerator is the
            // belt-and-suspenders backstop for cases where iced's
            // keyboard subscription doesn't catch the keypress
            // (focused widget capturing it, etc.).
            accel(cmd_or_ctrl(), Code::KeyP)),
        &MenuItem::with_id(ids::TOOLS_SEARCH,        "Search\u{2026}",   true, accel(cmd_or_ctrl(), Code::KeyF)),
        &MenuItem::with_id(ids::TOOLS_RANDOM_ALBUM,  "Random Album",      true, accel(Modifiers::ALT, Code::KeyR)),
        &PredefinedMenuItem::separator(),
        // Web-search shortcuts: pull up the current selection (or
        // now-playing track) on a third-party service. No keyboard
        // accelerator — these are mouse/menu-driven only.
        &MenuItem::with_id(ids::TOOLS_SEARCH_APPLE,   "Search Apple Music\u{2026}",        true, None),
        &MenuItem::with_id(ids::TOOLS_SEARCH_SPOTIFY, "Search Spotify\u{2026}",            true, None),
        &MenuItem::with_id(ids::TOOLS_SEARCH_YOUTUBE, "Search YouTube\u{2026}",            true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(ids::TOOLS_REFRESH,        "Refresh",                           true, accel(Modifiers::empty(), Code::F5)),
    ]).ok();
    menu.append(&tools).ok();

    // ── Help ───────────────────────────────────────────────────────────
    let help = Submenu::new("&Help", true);
    help.append_items(&[
        &MenuItem::with_id(ids::HELP_USER_GUIDE, "User Guide",          true, None),
        &MenuItem::with_id(ids::HELP_SHORTCUTS,  "Keyboard Shortcuts",  true, accel(Modifiers::empty(), Code::F1)),
    ]).ok();
    menu.append(&help).ok();

    menu
}

/// Translate a clicked menu ID to the matching `GuiMessage`.
///
/// Direct items resolve to a concrete `Action`. Context-sensitive items
/// (Similar / Related / Open in Library / Artist Bio / Random Album /
/// Refresh / Switch Library) route through `MenuKeyClick(KeyEvent)` so
/// the shared `key_input::handle_key` dispatcher picks the right thing
/// based on `AppState`. A handful of GUI-only popups (About, User Guide,
/// Keyboard Shortcuts, Toggle Cover Art) resolve to their own messages.
pub fn menu_event_for_id(id: &str) -> Option<GuiMessage> {
    let id: u32 = id.parse().ok()?;
    tracing::info!("muda menu click: id={id}");
    let msg = match id {
        // ── File ────────────────────────────────────────────────────────
        ids::FILE_ABOUT          => GuiMessage::ShowAbout,
        ids::FILE_SETTINGS       => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Settings))),
        ids::FILE_SWITCH_LIBRARY => GuiMessage::Action(Action::Search(SearchAction::OpenLibraryPicker)),
        ids::FILE_SIGN_OUT       => GuiMessage::Action(Action::Settings(SettingsAction::Logout)),
        ids::FILE_QUIT           => GuiMessage::Action(Action::System(SystemAction::Quit)),

        // ── View ────────────────────────────────────────────────────────
        ids::VIEW_BROWSE         => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Browse))),
        ids::VIEW_QUEUE          => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::Queue))),
        ids::VIEW_NOW_PLAYING    => GuiMessage::Action(Action::Navigation(NavigationAction::SetView(View::NowPlaying))),
        ids::VIEW_LIBRARY        => GuiMessage::Action(Action::Navigation(NavigationAction::set_category(BrowseCategory::Library))),
        ids::VIEW_PLAYLISTS      => GuiMessage::Action(Action::Navigation(NavigationAction::set_category(BrowseCategory::Playlists))),
        ids::VIEW_GENRES         => GuiMessage::Action(Action::Navigation(NavigationAction::set_category(BrowseCategory::AlbumGenres))),
        ids::VIEW_FOLDERS        => GuiMessage::Action(Action::Navigation(NavigationAction::set_category(BrowseCategory::Folders))),
        ids::VIEW_SIMILAR        => GuiMessage::MenuKeyClick(ctrl_char_key('m')),
        ids::VIEW_RELATED        => GuiMessage::MenuKeyClick(ctrl_char_key('r')),
        ids::VIEW_OPEN_IN_LIBRARY=> GuiMessage::MenuKeyClick(ctrl_char_key('j')),
        ids::VIEW_ARTIST_BIO     => GuiMessage::MenuKeyClick(function_key(4)),
        ids::VIEW_SORT_DEFAULT   => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::Default))),
        ids::VIEW_SORT_ARTIST    => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByArtist))),
        ids::VIEW_SORT_ALBUM     => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByAlbum))),
        ids::VIEW_SORT_TITLE     => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByTitle))),
        ids::VIEW_SORT_DURATION  => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::ByDuration))),
        ids::VIEW_SORT_SHUFFLE   => GuiMessage::Action(Action::Search(SearchAction::ApplyFocusedSortMode(ColumnSortMode::Shuffled))),
        ids::VIEW_REVERSE_SORT   => GuiMessage::Action(Action::Search(SearchAction::ReverseFocusedSortDirection)),
        ids::VIEW_GROUP_BY_ALBUM => GuiMessage::Action(Action::Search(SearchAction::ToggleFocusedColumnGrouping)),
        ids::VIEW_TOGGLE_COVER_ART => GuiMessage::ToggleCoverArt,
        ids::VIEW_SCROLLING_LAYOUT => GuiMessage::Action(Action::Settings(SettingsAction::ToggleMillerLayout)),
        ids::VIEW_TALL_MODE        => GuiMessage::Action(Action::Settings(SettingsAction::ToggleTallMode)),

        // ── Playback ────────────────────────────────────────────────────
        ids::PB_PLAY_PAUSE => GuiMessage::Action(Action::Playback(PlaybackAction::TogglePlayPause)),
        ids::PB_STOP       => GuiMessage::Action(Action::Playback(PlaybackAction::Stop)),
        ids::PB_PREV       => GuiMessage::Action(Action::Playback(PlaybackAction::Previous)),
        ids::PB_NEXT       => GuiMessage::Action(Action::Playback(PlaybackAction::Next)),
        ids::PB_SEEK_BACK  => GuiMessage::Action(Action::Playback(PlaybackAction::SeekRelative(-10_000))),
        ids::PB_SEEK_FWD   => GuiMessage::Action(Action::Playback(PlaybackAction::SeekRelative(10_000))),
        ids::PB_VOL_UP     => GuiMessage::Action(Action::Playback(PlaybackAction::VolumeUp)),
        ids::PB_VOL_DOWN   => GuiMessage::Action(Action::Playback(PlaybackAction::VolumeDown)),
        ids::PB_MUTE       => GuiMessage::Action(Action::Playback(PlaybackAction::ToggleMute)),

        // ── Queue ───────────────────────────────────────────────────────
        ids::Q_ENQUEUE      => GuiMessage::Action(Action::Queue(QueueAction::EnqueueSelection)),
        ids::Q_ENQUEUE_NEXT => GuiMessage::Action(Action::Queue(QueueAction::EnqueueSelectionNext)),
        ids::Q_SAVE         => GuiMessage::Action(Action::Queue(QueueAction::PromptSavePlaylist)),
        ids::Q_CLEAR        => GuiMessage::Action(Action::Queue(QueueAction::ClearQueue)),
        ids::Q_SHUFFLE      => GuiMessage::Action(Action::Queue(QueueAction::ToggleQueueShuffle)),

        // ── Queue → DJ Modes ────────────────────────────────────────────
        ids::Q_DJ_STRETCH   => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Stretch))),
        ids::Q_DJ_GEMINI    => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Gemini))),
        ids::Q_DJ_FREEZE    => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Freeze))),
        ids::Q_DJ_TWOFER    => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Twofer))),
        ids::Q_DJ_CONTEMPO  => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Contempo))),
        ids::Q_DJ_GROUPIE   => GuiMessage::Action(Action::Radio(crate::app::action::RadioAction::ToggleDjMode(crate::app::state::DjMode::Groupie))),

        // ── Queue → Remix tools ─────────────────────────────────────────
        ids::Q_REMIX_GEMINI       => GuiMessage::Action(Action::Queue(QueueAction::RemixGemini)),
        ids::Q_REMIX_TWOFER       => GuiMessage::Action(Action::Queue(QueueAction::RemixTwofer)),
        ids::Q_REMIX_STRETCH      => GuiMessage::Action(Action::Queue(QueueAction::RemixStretch)),
        ids::Q_REMIX_DOPPELGANGER => GuiMessage::Action(Action::Queue(QueueAction::RemixDoppelganger)),
        ids::Q_REMIX_SHUFFLE      => GuiMessage::Action(Action::Queue(QueueAction::RemixShuffle)),
        ids::Q_REMIX_UNDO_SHUFFLE => GuiMessage::Action(Action::Queue(QueueAction::RemixUndoShuffle)),

        // ── Radio (top-level) ───────────────────────────────────────────
        ids::RADIO_ARTIST       => GuiMessage::Action(Action::Search(SearchAction::OpenArtistRadioPicker)),
        ids::RADIO_ADVENTURE    => GuiMessage::Action(Action::Search(SearchAction::OpenAdventureLauncher)),
        ids::RADIO_STATIONS     => GuiMessage::OpenStationsPopup,

        // ── Tools ───────────────────────────────────────────────────────
        ids::TOOLS_PALETTE        => GuiMessage::OpenCommandPalette,
        ids::TOOLS_SEARCH         => GuiMessage::Action(Action::Search(SearchAction::OpenSearchPopup)),
        ids::TOOLS_ADVENTURE      => GuiMessage::Action(Action::Search(SearchAction::OpenAdventureLauncher)),
        ids::TOOLS_ARTIST_RADIO   => GuiMessage::Action(Action::Search(SearchAction::OpenArtistRadioPicker)),
        ids::TOOLS_RANDOM_ALBUM   => GuiMessage::MenuKeyClick(alt_char_key('r')),
        ids::TOOLS_SEARCH_APPLE   => GuiMessage::Action(Action::System(SystemAction::OpenExternalSearch { target: crate::services::external_search::SearchTarget::AppleMusic, query: None })),
        ids::TOOLS_SEARCH_SPOTIFY => GuiMessage::Action(Action::System(SystemAction::OpenExternalSearch { target: crate::services::external_search::SearchTarget::Spotify,    query: None })),
        ids::TOOLS_SEARCH_YOUTUBE => GuiMessage::Action(Action::System(SystemAction::OpenExternalSearch { target: crate::services::external_search::SearchTarget::YouTube,    query: None })),
        ids::TOOLS_REFRESH        => GuiMessage::MenuKeyClick(function_key(5)),

        // ── Help ────────────────────────────────────────────────────────
        ids::HELP_USER_GUIDE => GuiMessage::OpenUserGuide,
        ids::HELP_SHORTCUTS  => GuiMessage::OpenKeyboardShortcuts,

        _ => return None,
    };
    Some(msg)
}

// ── Synthetic-key helpers ───────────────────────────────────────────────

/// Build a crossterm `KeyEvent` for a function key `Fn`. Used for menu
/// items that dispatch through the shared key handler (refresh / bio).
fn function_key(n: u8) -> KeyEvent {
    KeyEvent {
        code: KeyCode::F(n),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// `Ctrl+<char>` synthetic event. The shared key handler matches on
/// `KeyModifiers::CONTROL`; on macOS the user actually presses
/// `Cmd+<char>` (the muda accelerator), and we synthesize the
/// equivalent CONTROL event so the same handler fires.
fn ctrl_char_key(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

/// `Alt+<char>` synthetic event. Used by Random Album (Alt+R).
fn alt_char_key(c: char) -> KeyEvent {
    KeyEvent {
        code: KeyCode::Char(c),
        modifiers: KeyModifiers::ALT,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

// ── Accelerator helpers ─────────────────────────────────────────────────

#[inline]
fn accel(mods: Modifiers, code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Some(mods), code))
}

/// `Cmd` on macOS, `Ctrl` elsewhere — for muda accelerators.
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
