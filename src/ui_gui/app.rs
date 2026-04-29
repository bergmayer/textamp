//! Iced `Application` — owns the shared `Core` (state + plex client + audio)
//! and drives it through the same `Action` dispatch the TUI uses.
//!
//! # Architecture
//! - The application owns its Core directly; `update()` mutates state by
//!   running the async dispatch inside `futures::executor::block_on`. The
//!   dispatchers themselves do not block on long-running I/O — they spawn
//!   tokio tasks internally that emit results back on `event_tx`.
//! - A tokio `mpsc` channel is the bridge. Async tasks emit `Event`s;
//!   `subscription()` turns those into `GuiMessage::CoreEvent` and feeds
//!   them back into `update()`.
//! - All real async work (Plex HTTP, waveform generation, etc.) runs via
//!   those spawned tasks on Iced's built-in tokio executor.
//!
//! The view function renders the current `state.view` through the
//! per-screen modules in `crate::ui_gui::screens`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use futures::SinkExt;
use iced::keyboard;
use iced::widget::{column, container};
use iced::{Element, Length, Subscription, Task, Theme};
use tokio::sync::mpsc;

use crate::app::{dispatch, Action, AppState, Event, View};
use crate::app::state::ConnectionState;
use crate::audio::AudioPlayer;
use crate::config::{self, Config};
use crate::plex::{PlexAuth, PlexClient, PlexClientInfo};

use super::message::GuiMessage;
use super::theme::iced_theme;
use super::viewport::Viewport;
use super::widgets::menu_bar::{self, TopMenu};

/// Bundles the mutable runtime state a dispatch needs to borrow.
///
/// Held inline by [`App`] (not behind an `Arc<Mutex>`) because Iced
/// serialises `update()` calls, giving us exclusive access for the duration
/// of each message.
struct Core {
    state: AppState,
    client: PlexClient,
    audio: AudioPlayer,
    config: Config,
    event_tx: mpsc::Sender<Event>,
}

/// Shared holder for the event receiver.
///
/// Iced's `subscription()` is called through `&self`, so we can't consume the
/// receiver out of a plain `Option<Receiver>` on the struct. We wrap it in
/// `Arc<Mutex<Option<_>>>` and `take()` inside the subscription the first
/// time it runs. Iced caches the subscription by `id`, so `take()` fires
/// exactly once.
type EventRxHolder = Arc<Mutex<Option<mpsc::Receiver<Event>>>>;

/// Mutually-exclusive primary popup. At most one of these can be open
/// at any moment — opening a different one closes the previous one
/// automatically by virtue of the field assignment.
///
/// `art_popup_key`, `menu_open`, and `context_menu` are kept as
/// separate fields on `App` because they layer over a primary popup
/// (e.g. right-click while Similar is open) rather than replacing it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrimaryPopup {
    #[default]
    None,
    About,
    Stations,
    DjModes,
    RemixTools,
    UserGuide,
    KeyboardShortcuts,
    Settings,
    Similar,
    Related,
}

impl PrimaryPopup {
    /// True iff a primary popup is currently shown. Useful for any
    /// render path that needs "is anything modal up right now?".
    #[allow(dead_code)]
    pub fn is_open(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Cached scroll state for a single Miller column scrollable. Updated
/// from the scrollable's `on_scroll` callback and read when computing a
/// natural scroll snap on keyboard navigation.
#[derive(Debug, Clone, Copy, Default)]
struct ColumnScroll {
    offset_y: f32,
    bounds_h: f32,
    content_h: f32,
}

/// Top-level Iced state.
pub struct App {
    core: Core,
    event_rx: EventRxHolder,
    viewport: Viewport,
    /// Which top-level menu (if any) is currently open.
    menu_open: Option<TopMenu>,
    /// At most one mutually-exclusive primary popup is open at a
    /// time — this enum encodes which (if any). The render path
    /// matches on it to layer the popup over the main view; the ESC
    /// handler dismisses it via `PrimaryPopup::dismiss`.
    ///
    /// `art_popup_key`, `menu_open`, and `context_menu` are kept
    /// separate because they're transient overlays that *compose*
    /// over the primary popup (right-click while the Similar popup
    /// is open, etc.), not alternatives to it.
    primary_popup: PrimaryPopup,
    /// Per-column scroll state reported by each Miller column's scrollable
    /// `on_scroll`. Enables natural "only scroll when selection would go
    /// off-screen" behaviour on arrow-key navigation.
    scroll_state: HashMap<usize, ColumnScroll>,
    /// Last window geometry persisted to disk. Used by the resize handler
    /// to dedupe redundant writes when the OS emits same-size events.
    last_saved_window: (u32, u32),

    /// Last known cursor position in logical coordinates. Updated by an
    /// app-wide `mouse_area` wrapping the main view and consumed when a
    /// right-click needs to position its context menu.
    mouse_pos: (f32, f32),

    /// Active context menu (right-click popup). None when no menu is open.
    context_menu: Option<super::widgets::context_menu::ContextMenuState>,

    /// Queue drag-and-drop reorder state. `Some((current_idx, moved))`
    /// while the user holds left-button and the cursor is over the
    /// queue track list. `current_idx` is the row's CURRENT position
    /// after live reorders triggered by drag-over events; `moved` is
    /// true once at least one reorder has happened. When the user
    /// releases:
    ///   - `moved == false` → it was a click, dispatch play.
    ///   - `moved == true`  → it was a drag, the queue is already in
    ///                        its final order (each `QueueDragOver`
    ///                        committed an incremental move); just
    ///                        clear the state.
    queue_drag: Option<(usize, bool)>,

    /// Tick countdown for the audio self-heal retry. Ticks fire every
    /// 100 ms; while `audio_available` is false the tick handler
    /// decrements this counter and, when it hits 0, calls
    /// `try_attach_backend()` and resets the counter. Initial value
    /// is small so we recover quickly after startup; on each failed
    /// retry we reset to ~2 s to avoid hammering WASAPI.
    audio_retry_ticks_left: u32,

    /// Full-size album art popup. Holds the rating key of the album
    /// whose cover is currently being displayed; `None` when no popup.
    art_popup_key: Option<String>,

    /// Separate high-resolution artwork cache, keyed by rating key.
    /// Grid rows use `state.artwork.grid_cache` (small thumbs); the
    /// art popup prefers this hi-res cache when populated. Kept on the
    /// GUI app rather than `AppState` so the TUI doesn't pay for it.
    hires_art: HashMap<String, Vec<u8>>,

    /// Captured view to restore when Similar / Related popup closes.
    /// Set when the popup opens (variant of `PrimaryPopup`); read by
    /// the dismiss handler.
    similar_prev_view: Option<crate::app::state::View>,
    related_prev_view: Option<crate::app::state::View>,

    /// macOS only: have we installed the muda global main menu yet?
    /// Winit overwrites `NSApp.mainMenu` from inside its
    /// `applicationDidFinishLaunching:` hook, so installing the menu
    /// before iced's run loop starts gets clobbered. We defer until the
    /// first `update()` tick — which fires on the main thread *after*
    /// winit's launch hook — and then leak the muda wrapper so its Drop
    /// doesn't tear down the NSMenu NSApplication now retains.
    #[cfg(all(target_os = "macos", feature = "native-menus"))]
    macos_menu_installed: bool,

    /// Timestamp of the last user-driven motion event (keyboard nav,
    /// mouse-wheel scroll, miller-row click). Used together with
    /// `state.artwork.suppress_loads` to throttle album-art fetches
    /// while the user is rapidly navigating: while motion has been
    /// happening within the last `ART_LOAD_PAUSE_MS`, every
    /// `LoadAlbumArt` is dropped; once motion settles for that long,
    /// the next tick clears the flag and dispatches a fresh load
    /// against the current viewport.
    last_motion_at: std::time::Instant,

    /// Live keyboard modifier state, updated from a window-level
    /// `keyboard::Event::ModifiersChanged` subscription. Mouse clicks
    /// don't carry modifier info in iced 0.13, so we cache it here and
    /// consult it when a click message arrives — the standard
    /// shift+click / cmd+click pattern.
    current_modifiers: iced::keyboard::Modifiers,

    /// "Anchor" row for queue range-select: the last row the user
    /// single-clicked or cmd-clicked. A subsequent shift-click selects
    /// the inclusive range between this anchor and the clicked row.
    /// `None` until the first click.
    queue_anchor: Option<usize>,

    /// Per-track cache of sonically-similar tracks shown in the
    /// Browse track-details pane. Populated lazily by an iced
    /// `Task::perform` from the Tick handler whenever the focused
    /// Miller track row changes to one we haven't seen yet.
    track_pane_similar: HashMap<String, Vec<crate::plex::models::Track>>,
    /// Track keys whose similar fetch is currently in flight — keeps
    /// the Tick handler from spawning duplicate requests.
    track_pane_similar_loading: std::collections::HashSet<String>,

    /// Persistent vectorscope state. Drained from the audio sample
    /// tap on each Tick when Now Playing → vectorscope is the active
    /// visualizer; canvas reads a clone of `samples` to draw the
    /// rolling Lissajous trace.
    vectorscope: super::widgets::vectorscope_canvas::Vectorscope,
    /// Clone of the rodio `SampleTap` (live-sample ring shared with
    /// the audio thread). `None` when no audio backend is available.
    vectorscope_tap: Option<crate::audio::SampleTap>,

}

/// Entry point called from `src/bin/textamp_gui.rs`.
pub fn run() -> Result<()> {
    // Install muda's process-wide menu-event handler before iced spins
    // up its tokio runtime + winit event loop. The handler forwards
    // every click into a tokio mpsc; the iced subscription drains that
    // mpsc. Doing this early means the subscription's first poll
    // finds the receiver waiting — if we deferred to `App::update` the
    // subscription would already have exited and re-subscriptions are
    // dedup'd by ID, so it would never read again.
    //
    // The matching `init_for_nsapp` call still has to wait until after
    // winit's `applicationDidFinishLaunching:` hook (see `App::update`).
    #[cfg(feature = "native-menus")]
    super::menu::install_event_forwarder();

    // Peek at the saved window size before Iced takes over — fall back to a
    // comfortable default when no config exists or it fails to parse.
    // `App::new` reads the config a second time; that's cheap and keeps the
    // two paths independent.
    let (win_w, win_h) = match config::load_config() {
        Ok(c) => (c.ui.window.width.max(720) as f32, c.ui.window.height.max(520) as f32),
        Err(_) => (1280.0, 840.0),
    };
    tracing::info!("Window: opening at {win_w}x{win_h} (from config).");
    let window = iced::window::Settings {
        // Default size is a touch larger so that the app feels comfortable
        // on a typical 1080p+ display after the 1.25× minimum scale factor
        // is applied below. Min size stays small enough for 720p laptops.
        size: iced::Size::new(win_w, win_h),
        min_size: Some(iced::Size::new(720.0, 520.0)),
        resizable: true,
        decorations: true,
        ..iced::window::Settings::default()
    };
    // Default font — match the platform's native system font so the
    // app looks like a first-class citizen, not a generic toolkit
    // window. Cosmic-text falls back through other installed fonts
    // for glyphs outside the base family (CJK / emoji / symbols).
    #[cfg(target_os = "windows")]
    let default_font = iced::Font::with_name("Segoe UI");
    #[cfg(target_os = "macos")]
    let default_font = iced::Font::with_name("SF Pro");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let default_font = iced::Font::with_name("sans-serif");

    let app_builder = iced::application(App::title, App::update, App::view)
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window)
        .scale_factor(App::scale_factor)
        .default_font(default_font);

    // Preload Windows fallback fonts so cosmic-text has CJK + symbol
    // + emoji glyph coverage when the base font (Segoe UI) is missing
    // a glyph. Segoe UI is Latin-only; without these, Japanese /
    // Chinese / Korean track names render as a row of squares (the
    // user's "I never want to see those gross unicode boxes" report)
    // and assorted dingbats (heavy multiplication X, music note,
    // bullets) drop out the same way.
    //
    // All four files ship with every modern Windows install, but if
    // any are missing we keep going — cosmic-text will render boxes
    // for the unmapped codepoints, which is no worse than today.
    #[cfg(target_os = "windows")]
    let app_builder = {
        const FALLBACK_FONTS: &[&str] = &[
            r"C:\Windows\Fonts\seguisym.ttf", // Segoe UI Symbol — dingbats, arrows, math
            r"C:\Windows\Fonts\seguiemj.ttf", // Segoe UI Emoji — emoji
            r"C:\Windows\Fonts\msyh.ttc",     // Microsoft YaHei — Simplified Chinese / shared CJK Han
            r"C:\Windows\Fonts\YuGothM.ttc",  // Yu Gothic Medium — Japanese kana + kanji
            r"C:\Windows\Fonts\malgun.ttf",   // Malgun Gothic — Korean Hangul
        ];
        let mut b = app_builder;
        for path in FALLBACK_FONTS {
            match std::fs::read(path) {
                Ok(bytes) => {
                    tracing::info!("Loaded fallback font: {path}");
                    b = b.font(bytes);
                }
                Err(e) => tracing::debug!("Skipping fallback font {path}: {e}"),
            }
        }
        b
    };

    // macOS analogue: SF Pro is Latin/Cyrillic/Greek-only, so without
    // these preloads cosmic-text renders boxes for emoji, dingbats,
    // and CJK glyphs (the user's "Fresh ❤" / "❤️ Tracks" report —
    // playlist names that came back as "Fresh [box]" instead).
    //
    // Apple Color Emoji is a CBDT-style colour bitmap font that iced
    // 0.13 / cosmic-text 0.12 will pick up for codepoints in the
    // emoji ranges; it just needs to be in the in-memory font db.
    #[cfg(target_os = "macos")]
    let app_builder = {
        const FALLBACK_FONTS: &[&str] = &[
            "/System/Library/Fonts/Apple Color Emoji.ttc",      // emoji (colour bitmap)
            "/System/Library/Fonts/Apple Symbols.ttf",          // misc symbols
            "/System/Library/Fonts/Symbol.ttf",                 // math / Greek
            "/System/Library/Fonts/ZapfDingbats.ttf",           // ✓ ✗ ❤ ★ etc.
            "/System/Library/Fonts/CJKSymbolsFallback.ttc",     // CJK punctuation
            "/System/Library/Fonts/Hiragino Sans GB.ttc",       // Simplified Chinese
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",       // Korean Hangul
            "/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc",  // Japanese kana + kanji
            "/System/Library/Fonts/GeezaPro.ttc",               // Arabic
        ];
        let mut b = app_builder;
        for path in FALLBACK_FONTS {
            match std::fs::read(path) {
                Ok(bytes) => {
                    tracing::info!("Loaded fallback font: {path}");
                    b = b.font(bytes);
                }
                Err(e) => tracing::debug!("Skipping fallback font {path}: {e}"),
            }
        }
        b
    };

    app_builder
        .run_with(App::new)
        .map_err(|e| anyhow::anyhow!("iced application failed: {e}"))
}

impl App {
    /// Open / close helpers for the primary popup. Doing the
    /// `primary_popup = ...` assignment via a method keeps every
    /// flip readable and lets `close_*` early-return if the popup
    /// wasn't the one currently open (avoids accidentally clobbering
    /// a different popup that opened in the meantime).
    fn open_about(&mut self)             { self.primary_popup = PrimaryPopup::About; }
    fn close_about(&mut self)            { self.close_if(PrimaryPopup::About); }
    fn is_about_open(&self) -> bool      { self.primary_popup == PrimaryPopup::About }

    fn open_stations(&mut self)          { self.primary_popup = PrimaryPopup::Stations; }
    fn close_stations(&mut self)         { self.close_if(PrimaryPopup::Stations); }
    fn is_stations_open(&self) -> bool   { self.primary_popup == PrimaryPopup::Stations }

    fn open_dj_modes(&mut self)          { self.primary_popup = PrimaryPopup::DjModes; }
    fn close_dj_modes(&mut self)         { self.close_if(PrimaryPopup::DjModes); }
    fn is_dj_modes_open(&self) -> bool   { self.primary_popup == PrimaryPopup::DjModes }

    fn open_remix_tools(&mut self)       { self.primary_popup = PrimaryPopup::RemixTools; }
    fn close_remix_tools(&mut self)      { self.close_if(PrimaryPopup::RemixTools); }
    fn is_remix_tools_open(&self) -> bool{ self.primary_popup == PrimaryPopup::RemixTools }

    fn open_user_guide(&mut self)        { self.primary_popup = PrimaryPopup::UserGuide; }
    fn close_user_guide(&mut self)       { self.close_if(PrimaryPopup::UserGuide); }
    fn is_user_guide_open(&self) -> bool { self.primary_popup == PrimaryPopup::UserGuide }

    fn open_keyboard_shortcuts(&mut self){ self.primary_popup = PrimaryPopup::KeyboardShortcuts; }
    fn close_keyboard_shortcuts(&mut self){ self.close_if(PrimaryPopup::KeyboardShortcuts); }
    fn is_keyboard_shortcuts_open(&self) -> bool { self.primary_popup == PrimaryPopup::KeyboardShortcuts }

    fn open_settings(&mut self)          { self.primary_popup = PrimaryPopup::Settings; }
    fn close_settings(&mut self)         { self.close_if(PrimaryPopup::Settings); }
    fn is_settings_open(&self) -> bool   { self.primary_popup == PrimaryPopup::Settings }

    fn open_similar(&mut self)           { self.primary_popup = PrimaryPopup::Similar; }
    fn close_similar(&mut self)          { self.close_if(PrimaryPopup::Similar); }
    fn is_similar_open(&self) -> bool    { self.primary_popup == PrimaryPopup::Similar }

    fn open_related(&mut self)           { self.primary_popup = PrimaryPopup::Related; }
    fn close_related(&mut self)          { self.close_if(PrimaryPopup::Related); }
    fn is_related_open(&self) -> bool    { self.primary_popup == PrimaryPopup::Related }

    fn close_if(&mut self, which: PrimaryPopup) {
        if self.primary_popup == which {
            self.primary_popup = PrimaryPopup::None;
        }
    }

    fn new() -> (Self, Task<GuiMessage>) {
        // Load configuration from the shared XDG path (same file the TUI uses).
        let config = match config::load_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to load config, using defaults: {}", e);
                Config::default()
            }
        };

        // Plex client uses the stored `client_identifier` so auth tokens
        // issued to previous sessions still work. Same logic as main.rs.
        let client_info = if let Some(stored) = PlexAuth::load_token() {
            let mut info = PlexClientInfo::default();
            info.client_identifier = stored.client_identifier;
            info
        } else {
            PlexClientInfo::default()
        };
        let client = PlexClient::new(client_info);

        // Audio player with a generous retry. Windows takes a while to
        // reassign the default WASAPI endpoint after a process that was
        // holding it exits — a force-kill plus relaunch cycle (the only
        // way to swap binaries while the user has the app open) can
        // leave the audio service rejecting `default_output_device()`
        // for several seconds. The retry budget here has to cover that
        // worst case so the user never sees a false "Audio unavailable"
        // banner on a hot rebuild. Total worst-case wait ~12 s; the
        // happy path returns instantly on the first attempt.
        let (audio, audio_error) = {
            let mut last_err: Option<String> = None;
            let mut player = None;
            // Backoff in ms: 250, 500, 750, 1000, 1500, 1500, 2000, 2000, 2500.
            // Sum = 12000 ms across 10 attempts (the first attempt is t=0).
            const BACKOFF_MS: &[u64] = &[250, 500, 750, 1000, 1500, 1500, 2000, 2000, 2500];
            for attempt in 0..=BACKOFF_MS.len() {
                match AudioPlayer::new() {
                    Ok(p) => { player = Some(p); break; }
                    Err(e) => {
                        last_err = Some(e.to_string());
                        if let Some(&ms) = BACKOFF_MS.get(attempt) {
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                        }
                    }
                }
            }
            match player {
                Some(p) => (p, None),
                None => {
                    let msg = format!(
                        "Audio unavailable: {} (playback disabled — Tools → Retry Audio Device)",
                        last_err.as_deref().unwrap_or("no default output device")
                    );
                    tracing::error!("{msg}");
                    (AudioPlayer::new_without_audio(), Some(msg))
                }
            }
        };

        let mut state = AppState::new();
        state.audio_available = audio.has_audio();
        state.playback.volume = config.playback.default_volume;
        state.transcode_kbps = config.playback.transcode_kbps;
        state.theme = crate::app::theme::ThemeName::from_config(&config.ui.theme);
        state.artwork.default_visible = config.ui.cover_art_view;
        state.artwork.mode = crate::app::state::ArtworkMode::from_config(&config.ui.artwork_mode);
        if let Some(err) = audio_error {
            state.set_error(err);
        }

        let (event_tx, event_rx) = mpsc::channel::<Event>(256);

        let mut audio = audio;
        audio.set_volume(config.playback.default_volume);

        let core = Core { state, client, audio, config, event_tx };
        let initial_window = (core.config.ui.window.width, core.config.ui.window.height);
        let mut app = Self {
            core,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            viewport: Viewport::default(),
            menu_open: None,
            primary_popup: PrimaryPopup::None,
            scroll_state: HashMap::new(),
            last_saved_window: initial_window,
            mouse_pos: (0.0, 0.0),
            context_menu: None,
            queue_drag: None,
            // Initial value 5 ticks (500 ms) so the first retry happens
            // soon after startup; subsequent failed retries throttle to
            // ~2 s in `handle_tick`. Set to 0 here only so the field is
            // initialized; the first retry waits 5 ticks because of the
            // saturating_sub at the top of the tick branch.
            audio_retry_ticks_left: 5,
            art_popup_key: None,
            #[cfg(all(target_os = "macos", feature = "native-menus"))]
            macos_menu_installed: false,
            // Initial artwork loads (driven by data-load follow-ups,
            // not user motion) run normally because `suppress_loads`
            // defaults to false. The timestamp only matters once the
            // user starts navigating.
            last_motion_at: std::time::Instant::now(),
            current_modifiers: iced::keyboard::Modifiers::empty(),
            queue_anchor: None,
            track_pane_similar: HashMap::new(),
            track_pane_similar_loading: std::collections::HashSet::new(),
            hires_art: HashMap::new(),
            similar_prev_view: None,
            related_prev_view: None,
            vectorscope: super::widgets::vectorscope_canvas::Vectorscope::default(),
            vectorscope_tap: None,
        };
        // Capture the rodio sample tap if the backend came up. Re-fetched
        // after `try_attach_backend` recoveries elsewhere.
        app.vectorscope_tap = app.core.audio.sample_tap();

        // Kick off authentication via the shared spawn_auth_task (same as
        // the TUI). It runs on iced's tokio executor and posts AuthEvent
        // results onto `event_tx`, which the subscription delivers back.
        app.core.state.connection = crate::app::state::ConnectionState::Authenticating;
        app.core.state.auth_state.step = crate::app::state::AuthStep::Checking;
        crate::app::dispatch::spawn_auth_task(app.core.event_tx.clone());

        (app, Task::none())
    }

    /// Iced content scale factor — reads the user-settable `ui_scale`
    /// from config. Fixed per session (not dynamic on window height) so
    /// saved window geometry round-trips cleanly through
    /// `window::Settings::size` and `resize_events`.
    fn scale_factor(&self) -> f64 {
        let s = self.core.config.ui.ui_scale
            .clamp(crate::config::settings::UI_SCALE_MIN, crate::config::settings::UI_SCALE_MAX);
        s as f64
    }

    fn title(&self) -> String {
        // Prefer the active library's title over the account username: the
        // library is what the user is actually looking at, and a single
        // account often owns several.
        if let Some(key) = &self.core.state.active_library {
            if let Some(lib) = self.core.state.libraries.iter().find(|l| &l.key == key) {
                return format!("Textamp — {}", lib.title);
            }
        }
        match &self.core.state.connection {
            ConnectionState::Connected { username, .. } => format!("Textamp — {}", username),
            _ => "Textamp".to_string(),
        }
    }

    /// Persist the current window dimensions to the shared config file.
    ///
    /// Scale factor is fixed at 1.0 (see `scale_factor`), so the resize
    /// event's width/height are already in the same units
    /// `window::Settings::size` expects — no conversion needed. Writes
    /// are cheap (tiny TOML + atomic rename) and dedup guards against
    /// same-size events fired on focus changes.
    fn maybe_persist_window_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == self.last_saved_window {
            return;
        }

        self.core.config.ui.window.width = width;
        self.core.config.ui.window.height = height;
        self.last_saved_window = (width, height);

        match config::save_config(&self.core.config) {
            Ok(()) => tracing::debug!("Window: saved size {width}x{height} to config."),
            Err(e) => tracing::warn!("Failed to save window size to config: {e}"),
        }
    }

    fn theme(&self) -> Theme {
        iced_theme(self.core.state.theme)
    }

    fn update(&mut self, message: GuiMessage) -> Task<GuiMessage> {
        // Lazy-art motion gate. Only rapid-fire navigation gestures
        // raise `suppress_loads` — the cases where the user is moving
        // the cursor faster than artwork can load:
        //   - keyboard arrow / page nav (`KeyPress`)
        //   - mouse-wheel scroll (`MillerScroll`)
        //   - alphabet-strip jumps (`AlphabetJump`)
        //
        // Mouse clicks on a row (`MillerSelect`) and column-focus
        // clicks (`FocusMillerColumn`) are *not* gated: those are
        // intentional "show me this content" gestures, and the user
        // expects the drilled-in column to populate its artwork
        // straight away. We even *clear* the flag on those events so
        // a click immediately after fast scrolling unblocks the
        // freshly-drilled column.
        match &message {
            GuiMessage::KeyPress(_)
            | GuiMessage::MillerScroll { .. }
            | GuiMessage::AlphabetJump(_) => {
                self.last_motion_at = std::time::Instant::now();
                self.core.state.artwork.suppress_loads = true;
            }
            GuiMessage::MillerSelect { .. } | GuiMessage::FocusMillerColumn { .. } => {
                self.core.state.artwork.suppress_loads = false;
            }
            _ => {}
        }

        // First-tick deferred install of the macOS global menu bar.
        // Runs on the main thread (iced calls `update` from winit's run
        // loop), and runs *after* winit's `applicationDidFinishLaunching:`
        // hook — which would otherwise overwrite the muda menu with its
        // own bare-bones default. The Menu wrapper is leaked so its Drop
        // doesn't tear down the NSMenu NSApplication now retains.
        #[cfg(all(target_os = "macos", feature = "native-menus"))]
        if !self.macos_menu_installed {
            tracing::info!("Installing macOS global muda menu (deferred from update)");
            let menu = super::menu::build();
            menu.init_for_nsapp();
            std::mem::forget(menu);
            self.macos_menu_installed = true;
        }

        let task = match message {
            GuiMessage::Noop => Task::none(),

            GuiMessage::WindowResized { width, height } => {
                self.viewport = Viewport { width, height };
                self.maybe_persist_window_size(width, height);
                Task::none()
            }

            GuiMessage::MenuOpen(menu) => {
                self.menu_open = Some(menu);
                Task::none()
            }

            GuiMessage::MenuClose => {
                self.menu_open = None;
                Task::none()
            }

            GuiMessage::MenuItemClick(action) => {
                self.menu_open = None;
                let prev = self.core.state.view;
                self.dispatch_sync(action);
                self.lift_view_to_popup(prev);
                Task::none()
            }

            GuiMessage::MenuKeyClick(key_event) => {
                self.menu_open = None;
                self.core.state.cache_mgmt.last_input_time = std::time::Instant::now();
                let prev = self.core.state.view;
                let actions = crate::app::handlers::key_input::handle_key(
                    key_event,
                    &mut self.core.state,
                    &self.core.config,
                );
                for a in actions {
                    self.dispatch_sync(a);
                }
                self.lift_view_to_popup(prev);
                Task::none()
            }

            GuiMessage::TabClick(actions) => {
                // TabClick is the generic "dismiss whatever dropdown or
                // popup launched me, then run these actions" path.
                self.menu_open = None;
                if self.is_similar_open() {
                    self.close_similar();
                    if let Some(v) = self.similar_prev_view.take() {
                        self.core.state.view = v;
                    }
                }
                if self.is_related_open() {
                    self.close_related();
                    if let Some(v) = self.related_prev_view.take() {
                        self.core.state.view = v;
                    }
                }
                let prev = self.core.state.view;
                for a in actions {
                    self.dispatch_sync(a);
                }
                self.lift_view_to_popup(prev);
                Task::none()
            }

            GuiMessage::ShowAbout => {
                self.menu_open = None;
                self.open_about();
                Task::none()
            }

            GuiMessage::HideAbout => {
                self.close_about();
                Task::none()
            }

            GuiMessage::ToggleCoverArt => {
                let new_visible = !self.core.state.artwork.default_visible;
                self.core.state.artwork.default_visible = new_visible;
                // Propagate to every nav's columns so existing views update
                // immediately, matching the TUI toggle_artwork() behaviour.
                for nav in [&mut self.core.state.artist_nav,
                            &mut self.core.state.genre_nav,
                            &mut self.core.state.playlist_nav] {
                    for col in nav.columns.iter_mut() {
                        col.artwork_visible = new_visible;
                    }
                }
                self.core.config.ui.cover_art_view = new_visible;
                if let Err(e) = config::save_config(&self.core.config) {
                    tracing::warn!("Failed to save cover_art_view: {e}");
                }
                Task::none()
            }

            GuiMessage::MillerScroll { column_index, offset_y, bounds_h, content_h } => {
                self.scroll_state.insert(
                    column_index,
                    ColumnScroll { offset_y, bounds_h, content_h },
                );
                // Lazy-load: as soon as the user scrolls past the
                // first viewport, queue the next page (and chain the
                // following pages until the playlist is fully loaded).
                // The previous "5 viewports from the bottom" heuristic
                // was too eager — for a 5991-row playlist that's a
                // hard-to-hit threshold and the rest of the list
                // appeared empty when grouping/artwork was on.
                let mut load_more: Option<(String, u32)> = None;
                if let Some(nav) = self.core.state.browse_nav() {
                    if let Some(col) = nav.columns.get(column_index) {
                        if let Some(lazy) = col.lazy.as_ref() {
                            let already = col.tracks.len() as u32;
                            let total = lazy.total.unwrap_or(u32::MAX);
                            let needs_more = already < total;
                            // Trigger when within two viewports of
                            // the bottom of the currently-loaded
                            // content; pages chain so we'll keep up.
                            let near_bottom = bounds_h > 0.0 && content_h > 0.0
                                && (offset_y + bounds_h) >= (content_h - bounds_h.max(400.0) * 2.0);
                            if needs_more && near_bottom && !lazy.loading {
                                load_more = Some((lazy.key.clone(), already));
                            }
                        }
                    }
                }
                if let Some((pk, off)) = load_more {
                    use crate::app::action::MillerAction;
                    self.dispatch_sync(Action::Miller(
                        MillerAction::LoadMorePlaylistTracks { playlist_key: pk, offset: off },
                    ));
                }

                // Also kick another art batch for the scrolled column
                // — the existing `collect_art_to_load` only windows
                // around `selected_index`, so as the user scrolls
                // away from it the new rows would otherwise stay
                // blank. `collect_all_art_to_load` is dedup-safe
                // (skips cached + pending) so calling it on every
                // scroll is cheap.
                let art_batch = if let Some(nav) = self.core.state.browse_nav() {
                    if let Some(col) = nav.columns.get(column_index) {
                        if col.artwork_visible {
                            crate::app::handlers::dispatch_miller::collect_all_art_to_load(
                                Some(col),
                                &self.core.state.artwork.grid_cache,
                                &self.core.state.artwork.grid_pending,
                            )
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                if !art_batch.is_empty() {
                    self.dispatch_sync(Action::System(
                        crate::app::action::SystemAction::LoadAlbumArt(art_batch),
                    ));
                }
                Task::none()
            }

            GuiMessage::FilterChanged(new_query) => {
                // The shared dispatcher handles activation, aliases, and
                // async execution. We just hand it the full edited value
                // — unlike the TUI's char-at-a-time keyboard path, a GUI
                // text_input always has the complete string.
                use crate::app::action::SearchAction;
                self.dispatch_sync(Action::Search(SearchAction::SetListFilterQuery(new_query)));
                Task::none()
            }

            GuiMessage::KeyPress(key_event) => self.handle_key_press(key_event),

            GuiMessage::Action(action) => {
                use crate::app::action::{NavigationAction, SettingsAction};
                tracing::info!("dispatch action from message: {action:?}");
                // Sign-out is the only action that should also dismiss
                // every GUI-only popup — otherwise the Settings panel
                // hovers above the auth screen post-logout, hiding the
                // sign-in form.
                let is_logout = matches!(&action, Action::Settings(SettingsAction::Logout));
                // Any action that re-stacks the Miller columns (category
                // switch, view change to/from Browse, jump-to-album)
                // makes our `scroll_state` cache stale: the scrollables
                // at miller-col-0 / miller-col-1 / … now contain a
                // different column. Dropping the cache on these
                // transitions complements the defensive clamp inside
                // `miller_column::view` so the new column starts at the
                // top no matter which path got us here.
                let resets_columns = matches!(
                    &action,
                    Action::Navigation(NavigationAction::SetCategory(_))
                        | Action::Navigation(NavigationAction::SetView(_))
                );
                let prev = self.core.state.view;
                self.dispatch_sync(action);
                if resets_columns {
                    self.scroll_state.clear();
                }
                if is_logout {
                    // Logout dismisses every primary popup so the
                    // sign-in form isn't covered by stale chrome.
                    self.primary_popup = PrimaryPopup::None;
                }
                self.lift_view_to_popup(prev);
                // Some actions (Open in Library, Jump to Album, …)
                // teleport between categories and re-stack the Miller
                // columns. Centre every column on its selected row
                // so the focused track at the deep end and the artist
                // / album rows above it all read as the same coherent
                // drilldown the user would have built by hand.
                self.center_all_columns_into_view()
            }

            GuiMessage::MillerSelect { column_index, item_index, activate } => {
                // Click on a Miller row claims focus from any other
                // surface (cat col, track-details pane). Single-focus
                // rule: only the column the user just clicked paints
                // as focused after this.
                self.core.state.category_column_focused = false;
                self.core.state.track_pane_focused = false;

                // Shift/cmd-click on a track row builds the column's
                // multi-selection set and SUPPRESSES the drill/play
                // actions — so the gesture is purely a selection
                // change. The set lives on `BrowseColumn::selected_set`;
                // the context menu reads it for "Play next / Add to
                // queue" bulk actions. Plain clicks clear the set.
                let mods = self.current_modifiers;
                let shift = mods.shift();
                let toggle_held = if cfg!(target_os = "macos") { mods.logo() } else { mods.control() };

                let multi_gesture = (shift || toggle_held) && {
                    let nav = self.core.state.browse_nav();
                    nav.and_then(|n| n.columns.get(column_index))
                        .map(|c| matches!(c.items.get(item_index), Some(crate::app::state::BrowseItem::Track { .. })))
                        .unwrap_or(false)
                };

                if multi_gesture {
                    if let Some(nav) = self.core.state.browse_nav_mut() {
                        if let Some(col) = nav.columns.get_mut(column_index) {
                            if shift {
                                let anchor = col.selection_anchor.unwrap_or(col.selected_index);
                                let (lo, hi) = if anchor <= item_index { (anchor, item_index) } else { (item_index, anchor) };
                                col.selected_set.clear();
                                for i in lo..=hi {
                                    col.selected_set.insert(i);
                                }
                            } else {
                                if !col.selected_set.remove(&item_index) {
                                    col.selected_set.insert(item_index);
                                }
                                col.selection_anchor = Some(item_index);
                            }
                            col.selected_index = item_index;
                        }
                        nav.focused_column = column_index;
                    }
                    tracing::info!(
                        "miller.click multi: col={column_index} idx={item_index} shift={shift} toggle={toggle_held}"
                    );
                    return Task::none();
                }

                // Plain click: clear any prior multi-selection on this
                // column and fall through to the existing drill/play
                // path.
                if let Some(nav) = self.core.state.browse_nav_mut() {
                    if let Some(col) = nav.columns.get_mut(column_index) {
                        col.selected_set.clear();
                        col.selection_anchor = Some(item_index);
                    }
                }

                let actions = miller_click_actions(&mut self.core.state, column_index, item_index, activate);
                let had_drill = !actions.is_empty();
                // Snapshot focused_column BEFORE dispatching drill
                // actions so we can detect whether a synchronous
                // push_column inside `miller_click_actions` (e.g. the
                // local grouped-album drill that returns an empty
                // actions vec) advanced focus to the new child.
                let pre_dispatch_focus = self
                    .core
                    .state
                    .browse_nav()
                    .map(|n| n.focused_column);
                for a in actions {
                    self.dispatch_sync(a);
                }
                let post_dispatch_focus = self
                    .core
                    .state
                    .browse_nav()
                    .map(|n| n.focused_column);
                let focus_advanced = pre_dispatch_focus != post_dispatch_focus
                    && post_dispatch_focus.map_or(false, |i| i > column_index);

                // Drilling via `push_column` advances the Miller focus
                // to the newly-added child column. The user expects
                // focus to follow the mouse instead, so the column
                // they clicked stays highlighted as the focused one.
                // Snap it back whenever we drilled — either via an
                // async action (`had_drill`) or synchronously (the
                // grouped-album path that does push_column locally
                // and returns an empty actions vec, detected via
                // `focus_advanced`).
                if had_drill || focus_advanced {
                    if let Some(nav) = self.core.state.browse_nav_mut() {
                        if column_index < nav.columns.len() {
                            nav.focused_column = column_index;
                        }
                    }
                    // The drill replaces the column at column_index+1
                    // (and may push deeper). Any cached scroll state for
                    // those slots is from a previous column at that
                    // index — leaving it in place makes the virtualizer
                    // hide the new column's first rows behind a stale
                    // top spacer. Drop them so the freshly-drilled
                    // column starts at offset 0.
                    self.scroll_state.retain(|&k, _| k <= column_index);
                    // Drilling adds a child column and can leave any
                    // ancestor's scroll position out of sync with its
                    // selected row (Iced's scrollable does not always
                    // preserve offset when the row tree changes shape).
                    // Re-centre every column on its current selection so
                    // an album view always looks like the user navigated
                    // to it themselves — artist visible in the artist
                    // column, album visible in the album column.
                    return self.center_all_columns_into_view();
                }
                Task::none()
            }

            GuiMessage::CoreEvent(ev) => {
                let actions = dispatch::handle_core_event(
                    ev,
                    &mut self.core.state,
                    &mut self.core.client,
                    &self.core.event_tx,
                );
                for a in actions {
                    self.dispatch_sync(a);
                }
                Task::none()
            }

            GuiMessage::Tick => {
                self.handle_tick();
                // Lazy-fetch sonic similars for the Browse track-pane.
                // Driven from the tick so a stale focused-row that's
                // missing its data eventually catches up — same
                // approach as the visualizer safety-net.
                self.maybe_fetch_track_pane_similar().unwrap_or_else(Task::none)
            }

            GuiMessage::SetVisualizerTab(tab) => {
                use crate::app::action::SystemAction;
                self.core.state.visualizer_tab = tab;
                // Kick off the matching data load. Waveform's loader
                // co-computes the spectrogram, so spectrum / spectrogram
                // are covered too. Vectorscope is live-audio, no bins
                // to pre-compute, so it skips the dispatch entirely.
                use crate::app::state::VisualizerTab;
                let load = match tab {
                    VisualizerTab::Waveform => Some(SystemAction::LoadWaveform),
                    VisualizerTab::Spectrum | VisualizerTab::Spectrogram =>
                        Some(SystemAction::LoadSpectrogram),
                    VisualizerTab::Vectorscope => None,
                };
                if let Some(action) = load {
                    self.dispatch_sync(Action::System(action));
                }
                Task::none()
            }

            GuiMessage::MouseMoved { x, y } => {
                self.mouse_pos = (x, y);
                Task::none()
            }

            GuiMessage::OpenMillerContextMenu { column_index, item_index } => {
                self.context_menu = build_miller_context_menu(
                    &self.core.state,
                    column_index,
                    item_index,
                    self.mouse_pos,
                );
                Task::none()
            }

            GuiMessage::OpenFolderContextMenu { row_index } => {
                self.context_menu = build_folder_context_menu(
                    &self.core.state,
                    row_index,
                    self.mouse_pos,
                );
                Task::none()
            }

            GuiMessage::FocusMillerColumn { column_index } => {
                // Click on a column's title bar — move keyboard focus
                // into the column. Don't touch its `selected_index`
                // (the user can click a row if they want to change
                // selection too) and don't truncate drill columns
                // to the right.
                if let Some(nav) = self.core.state.browse_nav_mut() {
                    if column_index < nav.columns.len() {
                        nav.focused_column = column_index;
                    }
                }
                self.core.state.category_column_focused = false;
                self.core.state.track_pane_focused = false;
                Task::none()
            }

            GuiMessage::FocusTrackPane => {
                self.core.state.track_pane_focused = true;
                self.core.state.category_column_focused = false;
                Task::none()
            }

            GuiMessage::SimilarRowClick { pane_index } => {
                use crate::app::action::BrowseAction;
                let was_selected = self.core.state.track_pane_focused
                    && self.core.state.track_pane_index == pane_index;
                self.core.state.track_pane_focused = true;
                self.core.state.track_pane_index = pane_index;
                self.core.state.category_column_focused = false;
                if was_selected {
                    // Already highlighted → drill into Library.
                    let parent = self.core.state.focused_track().cloned();
                    let sim_idx = pane_index.saturating_sub(1);
                    let sim = parent.as_ref().and_then(|t| {
                        self.track_pane_similar
                            .get(&t.rating_key)
                            .and_then(|v| v.get(sim_idx))
                            .cloned()
                    });
                    if let Some(sim) = sim {
                        if let Some(artist_key) = sim.grandparent_rating_key.clone() {
                            self.core.state.track_pane_focused = false;
                            self.core.state.track_pane_index = 0;
                            self.dispatch_sync(Action::Browse(BrowseAction::OpenInLibrary {
                                artist_key,
                                artist_name: sim.track_artist().to_string(),
                                album_key: sim.parent_rating_key.clone(),
                                album_title: sim.parent_title.clone(),
                            }));
                        }
                    }
                }
                Task::none()
            }

            GuiMessage::CloseMillerColumn { column_index } => {
                // Track-details pane "x": just clear the pane. The
                // close helper would otherwise also pop a column when
                // the pane was already absent, which the user did NOT
                // ask for from a click on the pane's own X.
                if column_index.is_none() {
                    self.core.state.track_details = None;
                    self.core.state.track_pane_focused = false;
                    self.core.state.track_pane_index = 0;
                    return Task::none();
                }
                // Miller column "x": focus the clicked column first so
                // the shared close helper drops the matching one.
                if let Some(idx) = column_index {
                    if state_browse_category_is_folders(&self.core.state) {
                        if let Some(fs) = self.core.state.folder_state.as_mut() {
                            if idx < fs.columns.len() {
                                fs.focused_column = idx;
                            }
                        }
                    } else if let Some(nav) = self.core.state.browse_nav_mut() {
                        if idx < nav.columns.len() {
                            nav.focused_column = idx;
                        }
                    }
                    self.core.state.category_column_focused = false;
                }
                crate::app::handlers::key_input::close_focused_browse_column(
                    &mut self.core.state,
                );
                Task::none()
            }

            GuiMessage::FolderRowClick { column_index, row_index, is_folder } => {
                use crate::app::action::FolderAction;
                // Pin folder nav focus + selection before any drill /
                // play dispatch — matches the per-row click semantics
                // the rest of the Miller stack uses (`MillerSelect`
                // does the same for non-folder navs).
                if let Some(fs) = self.core.state.folder_state.as_mut() {
                    if column_index < fs.columns.len() {
                        if let Some(col) = fs.columns.get_mut(column_index) {
                            if row_index < col.items.len() {
                                col.selected_index = row_index;
                            }
                        }
                        fs.focused_column = column_index;
                    }
                }
                let action = if is_folder {
                    let key = self.core.state.folder_state.as_ref()
                        .and_then(|fs| fs.columns.get(column_index))
                        .and_then(|c| c.items.get(row_index))
                        .map(|it| it.key.clone());
                    key.map(|k| Action::Folders(FolderAction::NavigateIntoFolder(k)))
                } else {
                    Some(Action::Folders(FolderAction::PlayFolderTrack { track_index: row_index }))
                };
                if let Some(a) = action {
                    self.dispatch_sync(a);
                }
                Task::none()
            }

            GuiMessage::ContextMenuClick(actions) => {
                self.context_menu = None;
                let prev = self.core.state.view;
                for a in actions {
                    self.dispatch_sync(a);
                }
                self.lift_view_to_popup(prev);
                // Right-click "Open in Library" / "Jump to Album"
                // teleport between categories — same column-coherence
                // contract as `GuiMessage::Action`. Without this call
                // the artist column ends up off-screen at the top
                // even though `selected_index` was set correctly.
                self.center_all_columns_into_view()
            }

            GuiMessage::CloseContextMenu => {
                self.context_menu = None;
                Task::none()
            }

            GuiMessage::SortPopupClick(idx) => {
                if let Some(p) = self.core.state.popups.sort.as_mut() {
                    if idx < p.options.len() {
                        p.selected_index = idx;
                    }
                }
                let actions = crate::app::handlers::key_input::sort_popup::apply_selected_option(
                    &mut self.core.state,
                );
                for a in actions {
                    self.dispatch_sync(a);
                }
                Task::none()
            }

            GuiMessage::RadioLauncherClick(idx) => {
                use crate::app::action::SearchAction;
                if let Some(rl) = self.core.state.popups.radio_launcher.as_mut() {
                    rl.item_index = idx;
                }
                self.dispatch_sync(Action::Search(SearchAction::RadioLauncherSelectResult));
                Task::none()
            }

            GuiMessage::AdventureLauncherClick(idx) => {
                use crate::app::action::SearchAction;
                if let Some(al) = self.core.state.popups.adventure_launcher.as_mut() {
                    al.item_index = idx;
                }
                self.dispatch_sync(Action::Search(SearchAction::AdventureLauncherSelectTrack));
                Task::none()
            }

            GuiMessage::ArtistRadioPickerClick(idx) => {
                use crate::app::action::SearchAction;
                if let Some(picker) = self.core.state.popups.artist_radio_picker.as_mut() {
                    picker.item_index = idx;
                }
                self.dispatch_sync(Action::Search(SearchAction::ArtistRadioPickerToggleArtist));
                Task::none()
            }

            GuiMessage::SearchPopupClick(idx) => {
                // Set the global search_item_index and fire the select
                // path — mirrors pressing Enter on the idx'th visible
                // result in the search popup. Centre the destination
                // column afterwards so the picked artist / album / etc.
                // is visible in the Miller stack, the same way the
                // Open-in-Library context-menu path does.
                use crate::app::action::SearchAction;
                self.core.state.list_state.search_item_index = idx;
                self.dispatch_sync(Action::Search(SearchAction::SelectSearchResult));
                self.center_all_columns_into_view()
            }

            GuiMessage::AuthUsernameChanged(s) => {
                self.core.state.auth_state.username_input = s;
                Task::none()
            }

            GuiMessage::AuthPasswordChanged(s) => {
                self.core.state.auth_state.password_input = s;
                Task::none()
            }

            GuiMessage::OpenStationsPopup => {
                self.open_stations();
                Task::none()
            }

            GuiMessage::CloseStationsPopup => {
                self.close_stations();
                Task::none()
            }

            GuiMessage::PlayStationAndClose(actions) => {
                self.close_stations();
                for a in actions {
                    self.dispatch_sync(a);
                }
                Task::none()
            }

            GuiMessage::OpenDjModesPopup => {
                self.open_dj_modes();
                Task::none()
            }

            GuiMessage::CloseDjModesPopup => {
                self.close_dj_modes();
                Task::none()
            }

            GuiMessage::OpenRemixToolsPopup => {
                self.open_remix_tools();
                Task::none()
            }

            GuiMessage::CloseRemixToolsPopup => {
                self.close_remix_tools();
                Task::none()
            }

            GuiMessage::RemixToolClick(action) => {
                self.close_remix_tools();
                self.dispatch_sync(action);
                Task::none()
            }

            GuiMessage::OpenUserGuide => {
                self.open_user_guide();
                self.menu_open = None;
                Task::none()
            }

            GuiMessage::CloseUserGuide => {
                self.close_user_guide();
                Task::none()
            }

            GuiMessage::OpenKeyboardShortcuts => {
                self.open_keyboard_shortcuts();
                self.menu_open = None;
                Task::none()
            }

            GuiMessage::CloseKeyboardShortcuts => {
                self.close_keyboard_shortcuts();
                Task::none()
            }

            GuiMessage::OpenArtPopup { key, thumb_path } => {
                self.art_popup_key = Some(key.clone());
                // Kick off a hi-res fetch if we don't already have one.
                // Plex transcodes to whatever size we request; 1600 is
                // sharp on a 1440p display and keeps the payload small.
                if !self.hires_art.contains_key(&key) {
                    let client = self.core.client.clone();
                    let k = key.clone();
                    let tp = thumb_path.clone();
                    return Task::perform(
                        async move {
                            match client.fetch_artwork(&tp, 1600).await {
                                Ok(data) => Some((k, data)),
                                Err(e) => {
                                    tracing::debug!("Hires art fetch failed: {e}");
                                    None
                                }
                            }
                        },
                        |opt| match opt {
                            Some((key, data)) => GuiMessage::HiresArtLoaded { key, data },
                            None => GuiMessage::Noop,
                        },
                    );
                }
                Task::none()
            }

            GuiMessage::CloseArtPopup => {
                self.art_popup_key = None;
                Task::none()
            }

            GuiMessage::HiresArtLoaded { key, data } => {
                self.hires_art.insert(key, data);
                Task::none()
            }

            GuiMessage::ShowSimilarPopup(actions) => {
                // Remember where we were so we can restore the view
                // when the popup closes — the LoadSimilar* dispatchers
                // flip `state.view` to `View::Similar`.
                let prev = self.core.state.view;
                for a in actions {
                    self.dispatch_sync(a);
                }
                self.similar_prev_view = if prev == crate::app::state::View::Similar {
                    None
                } else {
                    Some(prev)
                };
                self.core.state.view = prev; // snap back so the popup overlays the old view
                self.open_similar();
                // Close any context menu that launched us; the
                // context_menu widget's Custom branch doesn't auto-close.
                self.context_menu = None;
                Task::none()
            }

            GuiMessage::CloseSimilarPopup => {
                self.close_similar();
                if let Some(v) = self.similar_prev_view.take() {
                    self.core.state.view = v;
                }
                Task::none()
            }

            GuiMessage::ConfirmDialogYes => {
                use crate::app::action::SettingsAction;
                use crate::app::state::ConfirmAction;
                let dialog = self.core.state.popups.confirm_dialog.take();
                if let Some(d) = dialog {
                    let actions: Vec<Action> = match d.on_confirm {
                        ConfirmAction::RefreshCache => {
                            crate::app::handlers::helpers::refresh_current_view(&mut self.core.state)
                        }
                        ConfirmAction::ClearLibraryCache => vec![Action::Settings(SettingsAction::ClearLibraryCache)],
                        ConfirmAction::ClearArtworkCache => vec![Action::Settings(SettingsAction::ClearArtworkCache)],
                        ConfirmAction::ClearSubfolderCache => vec![Action::Settings(SettingsAction::ClearSubfolderCache)],
                        ConfirmAction::Quit => vec![Action::System(crate::app::action::SystemAction::Quit)],
                    };
                    for a in actions {
                        self.dispatch_sync(a);
                    }
                }
                Task::none()
            }

            GuiMessage::ConfirmDialogNo => {
                self.core.state.popups.confirm_dialog = None;
                Task::none()
            }

            GuiMessage::InputDialogChanged(s) => {
                if let Some(d) = self.core.state.popups.input_dialog.as_mut() {
                    d.input = s;
                }
                Task::none()
            }

            GuiMessage::InputDialogSubmit => {
                use crate::app::action::QueueAction;
                use crate::app::state::InputDialogAction;
                let dialog = self.core.state.popups.input_dialog.take();
                if let Some(d) = dialog {
                    match d.action_type {
                        InputDialogAction::SavePlaylist => {
                            self.dispatch_sync(Action::Queue(QueueAction::SaveQueueAsPlaylist(d.input)));
                        }
                        InputDialogAction::AdventureLength => {
                            // Adventure length is numeric; current state
                            // goes through the Adventure launcher popup
                            // rather than a free-text input. Stub here
                            // until the handler is needed.
                        }
                    }
                }
                Task::none()
            }

            GuiMessage::InputDialogCancel => {
                self.core.state.popups.input_dialog = None;
                Task::none()
            }

            GuiMessage::CloseBioPopup => {
                self.core.state.popups.artist_bio = None;
                Task::none()
            }

            GuiMessage::MoveQueueRowUp(idx) => {
                use crate::app::action::QueueAction;
                self.context_menu = None;
                self.core.state.list_state.queue_index = idx;
                self.dispatch_sync(Action::Queue(QueueAction::MoveQueueTrackUp));
                Task::none()
            }

            GuiMessage::MoveQueueRowDown(idx) => {
                use crate::app::action::QueueAction;
                self.context_menu = None;
                self.core.state.list_state.queue_index = idx;
                self.dispatch_sync(Action::Queue(QueueAction::MoveQueueTrackDown));
                Task::none()
            }

            GuiMessage::RemoveQueueRow(idx) => {
                use crate::app::action::QueueAction;
                self.dispatch_sync(Action::Queue(QueueAction::RemoveFromQueue(idx)));
                Task::none()
            }

            GuiMessage::ModifiersChanged(mods) => {
                self.current_modifiers = mods;
                Task::none()
            }

            GuiMessage::TrackPaneSimilarLoaded { track_key, tracks } => {
                self.track_pane_similar_loading.remove(&track_key);
                self.track_pane_similar.insert(track_key, tracks);
                Task::none()
            }

            GuiMessage::OpenStandaloneTrackContextMenu(track) => {
                self.context_menu = build_track_context_menu(
                    &self.core.state,
                    *track,
                    self.mouse_pos,
                );
                Task::none()
            }

            GuiMessage::OpenPlaylistFromCategory { playlist_key, title } => {
                use crate::app::action::{MillerAction, NavigationAction};
                use crate::app::state::BrowseCategory;
                // Switch to the Playlists nav, snap the root column's
                // selection to the clicked playlist, then drill into
                // its tracks. Mirrors the behaviour the user would get
                // by clicking the row inside the Playlists category
                // column — but skipping the "first click on Playlists,
                // second click on the row" two-step.
                self.dispatch_sync(Action::Navigation(NavigationAction::SetCategory(
                    BrowseCategory::Playlists,
                )));
                if let Some(col) = self.core.state.playlist_nav.columns.get_mut(0) {
                    if let Some(idx) = col.items.iter()
                        .position(|i| i.key() == playlist_key.as_str())
                    {
                        col.selected_index = idx;
                    }
                }
                self.core.state.playlist_nav.focused_column = 0;
                self.core.state.playlist_nav.truncate_right();
                self.core.state.library.selected_album_title = title;
                self.dispatch_sync(Action::Miller(
                    MillerAction::LoadPlaylistTracksForMiller { playlist_key },
                ));
                self.scroll_state.clear();
                self.center_all_columns_into_view()
            }

            GuiMessage::PlayOneRandomAlbum => {
                use rand::seq::IteratorRandom;
                use crate::app::action::QueueAction;
                let pick = self
                    .core.state.library.albums
                    .iter()
                    .choose(&mut rand::thread_rng())
                    .map(|a| (a.rating_key.clone(), a.title.clone()));
                if let Some((rating_key, title)) = pick {
                    self.dispatch_sync(Action::Queue(QueueAction::PlayAlbumNow {
                        rating_key,
                        title,
                    }));
                } else {
                    self.core.state.set_error("No albums in library to pick from".to_string());
                }
                Task::none()
            }

            GuiMessage::QueueDragStart(idx) => {
                // Standard desktop multi-select semantics, applied to
                // `state.queue.selected` so the existing shared
                // `RemoveSelectedFromQueue` / `MoveSelectedTracksUp/Down`
                // handlers fire over the right rows:
                //
                //   plain click    → clear selection, move cursor, set anchor
                //   shift+click    → range from anchor (or cursor) to clicked
                //   cmd/ctrl+click → toggle this row, set anchor
                //
                // The actual play/reorder decision still happens on
                // QueueDragEnd; a drag is detected by any subsequent
                // QueueDragOver landing on a different row. We
                // suppress the drag wiring entirely on shift/cmd
                // clicks so a multi-select gesture doesn't accidentally
                // become a reorder.
                let mods = self.current_modifiers;
                let shift = mods.shift();
                let toggle = if cfg!(target_os = "macos") { mods.logo() } else { mods.control() };
                tracing::info!("queue.click: idx={idx} shift={shift} toggle={toggle}");

                if shift {
                    let anchor = self
                        .queue_anchor
                        .unwrap_or(self.core.state.list_state.queue_index);
                    let (lo, hi) = if anchor <= idx { (anchor, idx) } else { (idx, anchor) };
                    self.core.state.queue.selected.clear();
                    for i in lo..=hi {
                        self.core.state.queue.selected.insert(i);
                    }
                    self.core.state.list_state.queue_index = idx;
                    // anchor stays put — that's how shift+shift+shift extends.
                    self.queue_drag = None;
                } else if toggle {
                    if !self.core.state.queue.selected.remove(&idx) {
                        self.core.state.queue.selected.insert(idx);
                    }
                    self.core.state.list_state.queue_index = idx;
                    self.queue_anchor = Some(idx);
                    self.queue_drag = None;
                } else {
                    self.core.state.queue.selected.clear();
                    self.core.state.list_state.queue_index = idx;
                    self.queue_anchor = Some(idx);
                    self.queue_drag = Some((idx, false));
                }
                Task::none()
            }

            GuiMessage::QueueDragOver(idx) => {
                // Live reorder: if the cursor crossed onto a different
                // row, immediately move the dragged track to that row
                // so the user sees it follow the cursor in real time
                // (instead of "teleporting" on release). Each crossing
                // dispatches one `MoveQueueTrack`; the queue ends up
                // in its final order by the time the user releases.
                if let Some((cur, moved)) = self.queue_drag {
                    if idx != cur {
                        tracing::info!("queue.drag: live move {cur} -> {idx}");
                        use crate::app::action::QueueAction;
                        self.dispatch_sync(Action::Queue(
                            QueueAction::MoveQueueTrack { from: cur, to: idx },
                        ));
                        self.queue_drag = Some((idx, true));
                    } else {
                        self.queue_drag = Some((cur, moved));
                    }
                }
                Task::none()
            }

            GuiMessage::QueueDragEnd => {
                tracing::info!("queue.drag: end (state: {:?})", self.queue_drag);
                if let Some((idx, moved)) = self.queue_drag.take() {
                    use crate::app::action::{QueueAction, RadioAction};
                    use crate::app::state::PlaybackMode;
                    if !moved {
                        // No drag → click → play / jump.
                        let action = match self.core.state.playback_mode {
                            PlaybackMode::Radio => Action::Radio(RadioAction::JumpToRadioTrack(idx)),
                            _ => Action::Queue(QueueAction::PlayTrackFromCategory(idx)),
                        };
                        self.dispatch_sync(action);
                    }
                    // moved == true: queue already in final order from
                    // the live reorders during drag — nothing to commit.
                }
                Task::none()
            }

            GuiMessage::OpenQueueContextMenu { row_index } => {
                self.core.state.list_state.queue_index = row_index;
                self.context_menu = build_queue_context_menu(
                    &self.core.state,
                    row_index,
                    self.mouse_pos,
                );
                Task::none()
            }

            GuiMessage::AlphabetJump(ch) => {
                // Letter-strip clicks are pure scroll actions: they
                // bring the matching row into view in the root column
                // but DO NOT change which artist is selected and DO
                // NOT close drilled-in child columns. Selection only
                // changes when the user actually clicks a row.
                use crate::ui_gui::widgets::miller_column::{row_height_for, scroll_id_for};
                use iced::widget::scrollable::{snap_to, RelativeOffset};
                let Some(target_idx) = alphabet_target_index(&self.core.state, ch) else {
                    return Task::none();
                };
                let Some(nav) = self.core.state.browse_nav() else { return Task::none() };
                let Some(col) = nav.columns.first() else { return Task::none() };
                if col.items.is_empty() { return Task::none() };

                // Pixel-accurate scroll: sum row heights up to the
                // target, place that y at viewport top so the matched
                // row is the first visible (alphabet jump = "scroll
                // here", not "centre here", since the user's eye
                // expects the letter group to start at the top).
                let show_art = col.artwork_visible;
                let mut target_top = 0.0_f32;
                for (i, item) in col.items.iter().enumerate() {
                    if i == target_idx { break; }
                    target_top += row_height_for(item, show_art);
                }
                let total_h: f32 = col.items.iter()
                    .map(|it| row_height_for(it, show_art))
                    .sum();
                let bounds_h = self.scroll_state.get(&0)
                    .map(|s| s.bounds_h)
                    .unwrap_or(0.0);
                let max_off = (total_h - bounds_h).max(0.0);
                let target = target_top.clamp(0.0, max_off);
                let rel_y = if max_off <= 0.0 {
                    0.0
                } else {
                    (target / max_off).clamp(0.0, 1.0)
                };
                snap_to(scroll_id_for(0), RelativeOffset { x: 0.0, y: rel_y })
            }

            GuiMessage::ShowRelatedPopup(actions) => {
                let prev = self.core.state.view;
                for a in actions {
                    self.dispatch_sync(a);
                }
                self.related_prev_view = if prev == crate::app::state::View::Related {
                    None
                } else {
                    Some(prev)
                };
                self.core.state.view = prev;
                self.open_related();
                self.context_menu = None;
                Task::none()
            }

            GuiMessage::CloseRelatedPopup => {
                self.close_related();
                if let Some(v) = self.related_prev_view.take() {
                    self.core.state.view = v;
                }
                Task::none()
            }

            GuiMessage::OpenSettingsPopup => {
                use crate::app::action::SettingsAction;
                self.open_settings();
                // Re-poll on-disk cache sizes so the Settings → Cache
                // table shows fresh numbers the moment the popup
                // opens. Without this the Size column reads "—"
                // until the user signs in / out again (the only other
                // place stats are computed).
                self.dispatch_sync(Action::Settings(SettingsAction::RefreshCacheStats));
                Task::none()
            }

            GuiMessage::CloseSettingsPopup => {
                self.close_settings();
                Task::none()
            }

            GuiMessage::SetSettingsSection(sec) => {
                use crate::app::action::SettingsAction;
                self.core.state.settings_state.section = sec;
                self.core.state.settings_state.item_index = 0;
                // Refresh on-disk cache figures whenever the user
                // lands on the Cache tab — covers the case where
                // `OpenSettingsPopup` fired before the library
                // finished loading, so the estimate measured an
                // empty in-memory state.
                if matches!(sec, crate::app::state::SettingsSection::Cache) {
                    self.dispatch_sync(Action::Settings(SettingsAction::RefreshCacheStats));
                }
                Task::none()
            }

            GuiMessage::SetTheme(name) => {
                self.core.state.theme = name;
                self.core.config.ui.theme = name.config_name().to_string();
                if let Err(e) = config::save_config(&self.core.config) {
                    tracing::warn!("Failed to save theme: {e}");
                }
                Task::none()
            }

            GuiMessage::RetryAudio => {
                match self.core.audio.try_attach_backend() {
                    Ok(true) => {
                        self.core.state.audio_available = true;
                        self.core.state.notifications.last_error = None;
                        self.core.state.set_status("Audio device connected.".to_string());
                    }
                    Ok(false) => {
                        // Backend already attached — nothing to do.
                    }
                    Err(e) => {
                        self.core.state.set_error(format!(
                            "Audio still unavailable: {} (check your output device and try again)",
                            e
                        ));
                    }
                }
                Task::none()
            }

            GuiMessage::NavigateToArtist { artist_key } => {
                use crate::app::action::{MillerAction, NavigationAction};
                use crate::app::state::{BrowseCategory, View};
                // Close any popup overlays so the user lands on the
                // library-drilled view, not behind a dimmer.
                self.close_similar();
                self.similar_prev_view = None;
                self.close_related();
                self.related_prev_view = None;
                // Category column should not claim focus — the drill
                // target is the albums column we're about to create.
                self.core.state.category_column_focused = false;

                // Set Library category + Browse view, then select the
                // artist in the nav and drill in. We set the
                // selected_index in artist_nav.columns[0] BEFORE the
                // drill so the parent-column highlight matches the
                // artist whose albums are being shown.
                self.dispatch_sync(Action::Navigation(NavigationAction::SetCategory(BrowseCategory::Library)));
                self.dispatch_sync(Action::Navigation(NavigationAction::SetView(View::Browse)));
                if let Some(col) = self.core.state.artist_nav.columns.get_mut(0) {
                    if let Some(pos) = col.items.iter().position(|i| i.key() == artist_key.as_str()) {
                        col.selected_index = pos;
                    }
                }
                // Reset focus to column 0, truncate anything further,
                // then drill. `LoadArtistAlbumsForMiller` uses
                // `push_column` which advances focus to the new child,
                // so the visible result is: artist column (grey-
                // highlighted selection) + albums column (cyan focus).
                self.core.state.artist_nav.focused_column = 0;
                self.core.state.artist_nav.truncate_right();
                // Update display name so the albums column header reads
                // correctly. Mirrors what `select_search_result` does.
                if let Some(name) = self
                    .core.state.artist_nav.columns.first()
                    .and_then(|c| c.items.get(c.selected_index))
                    .map(|i| i.title().to_string())
                {
                    self.core.state.library.selected_artist_name = name;
                }
                self.dispatch_sync(Action::Miller(MillerAction::LoadArtistAlbumsForMiller { artist_key }));
                Task::none()
            }

            GuiMessage::CloseStatePopup(kind) => {
                use super::message::StatePopupKind;
                match kind {
                    StatePopupKind::Sort => self.core.state.popups.sort = None,
                    StatePopupKind::Search => self.core.state.popups.search_active = false,
                    StatePopupKind::RadioLauncher => self.core.state.popups.radio_launcher = None,
                    StatePopupKind::AdventureLauncher => self.core.state.popups.adventure_launcher = None,
                    StatePopupKind::ArtistRadioPicker => self.core.state.popups.artist_radio_picker = None,
                }
                Task::none()
            }

            GuiMessage::AdjustUiScale(delta) => {
                use crate::config::settings::{UI_SCALE_MAX, UI_SCALE_MIN};
                let new_scale = (self.core.config.ui.ui_scale + delta)
                    .clamp(UI_SCALE_MIN, UI_SCALE_MAX);
                // Round to the nearest 0.05 so repeated +/- clicks don't
                // accumulate float drift.
                let new_scale = (new_scale * 20.0).round() / 20.0;
                if (new_scale - self.core.config.ui.ui_scale).abs() > f32::EPSILON {
                    self.core.config.ui.ui_scale = new_scale;
                    if let Err(e) = config::save_config(&self.core.config) {
                        tracing::warn!("Failed to save UI scale: {e}");
                    }
                }
                Task::none()
            }
        };

        // Honour the shared core's quit signal. Anything that flips
        // `state.should_quit` (Cmd+Q in the muda menu, File→Quit,
        // Ctrl+Q on Linux, Confirm-Dialog "yes" on quit, …) lands
        // here; we chain `iced::exit()` after the dispatched task so
        // the close still has a chance to run any pending side
        // effects (cache save, Plex stop report, etc.) first.
        if self.core.state.should_quit {
            return Task::batch([task, iced::exit()]);
        }
        task
    }

    fn view(&self) -> Element<'_, GuiMessage> {
        // Logical viewport width in pixels. Browse uses it to decide how
        // many Miller columns can fit side-by-side. Before the first
        // resize event fires, fall back to the initial window width from
        // config (scale_factor is fixed at 1.0 so no conversion needed).
        let viewport_w_logical = if self.viewport.width > 0 {
            self.viewport.width as f32
        } else {
            (self.core.config.ui.window.width as f32).max(600.0)
        };

        // View routing by `state.view` mirrors the TUI structure.
        let body: Element<'_, GuiMessage> = match self.core.state.view {
            View::Auth => super::screens::auth::view(&self.core.state),
            View::Browse => {
                let scroll_state = &self.scroll_state;
                super::screens::browse::view(
                    &self.core.state,
                    viewport_w_logical,
                    move |idx| match scroll_state.get(&idx) {
                        Some(s) => (s.offset_y, s.bounds_h),
                        None => (0.0, 0.0),
                    },
                    &self.track_pane_similar,
                )
            }
            View::Queue | View::NowPlaying => super::screens::queue::view(
                &self.core.state,
                self.queue_drag.map(|(src, _)| src),
                self.is_stations_open(),
                self.is_dj_modes_open(),
                self.is_remix_tools_open(),
                &self.vectorscope,
            ),
            View::Similar => super::screens::similar::view(&self.core.state),
            View::Related => super::screens::related::view(&self.core.state),
            View::Help => super::screens::help::view(&self.core.state),
            View::Settings => super::screens::settings::view(&self.core.state, self.core.config.ui.ui_scale),
            View::Search => super::screens::search::view(&self.core.state),
        };

        let transport = super::widgets::transport_bar::view(&self.core.state);

        // Hide menu while the user is signing in — items would be
        // visually noisy on the auth screen. Tabs are inside the
        // transport bar now and only shown when chrome is shown.
        let show_chrome = !matches!(self.core.state.view, View::Auth);
        // On macOS with native menus, the global menu bar at the top of
        // the screen is the menu — there must be no second one attached
        // to the window. Skip the in-window strip and dropdown entirely.
        const IN_WINDOW_MENU: bool = !cfg!(all(target_os = "macos", feature = "native-menus"));
        let menu_strip: Element<'_, GuiMessage> = if show_chrome && IN_WINDOW_MENU {
            menu_bar::bar(self.menu_open, &self.core.state)
        } else {
            iced::widget::Space::with_height(Length::Fixed(0.0)).into()
        };
        let transport_el: Element<'_, GuiMessage> = if show_chrome {
            transport
        } else {
            iced::widget::Space::with_height(Length::Fixed(0.0)).into()
        };

        let base: Element<'_, GuiMessage> = container(
            column![menu_strip, body, transport_el].spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        // Menu dropdown floats over content without pushing it down.
        let with_menu: Element<'_, GuiMessage> = if show_chrome && IN_WINDOW_MENU {
            if let Some(dropdown) = menu_bar::dropdown_overlay(self.menu_open, &self.core.state) {
                iced::widget::stack![base, dropdown].into()
            } else {
                base
            }
        } else {
            base
        };

        let with_popups = super::screens::popups::overlay(&self.core.state, with_menu);

        let with_about: Element<'_, GuiMessage> = if self.is_about_open() {
            iced::widget::stack![with_popups, super::screens::popups::about::view()].into()
        } else {
            with_popups
        };

        // Full-size album art popup — triggered by clicking a thumbnail
        // in a Miller row. Prefers the hi-res cache, falls back to the
        // grid thumbnail bytes while the hi-res fetch is in flight.
        let with_art_popup: Element<'_, GuiMessage> = if let Some(key) = self.art_popup_key.as_ref() {
            let bytes: Option<&[u8]> = self.hires_art.get(key).map(|v| v.as_slice())
                .or_else(|| self.core.state.artwork.grid_cache.get(key).map(|v| v.as_slice()));
            let img: Element<'_, GuiMessage> = match bytes {
                Some(b) => iced::widget::image(super::images::handle_from_bytes(b))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Contain)
                    .into(),
                None => iced::widget::text("Loading art\u{2026}").size(16).into(),
            };
            let frame = iced::widget::container(img)
                .width(Length::Fixed(viewport_w_logical.min(900.0)))
                .height(Length::Fixed(viewport_w_logical.min(900.0)))
                .padding(8)
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    iced::widget::container::Style {
                        background: Some(iced::Background::Color(p.background.base.color)),
                        border: iced::Border { color: p.primary.strong.color, width: 1.5, radius: 6.0.into() },
                        ..iced::widget::container::Style::default()
                    }
                });
            iced::widget::stack![
                with_about,
                modal_overlay(frame.into(), GuiMessage::CloseArtPopup),
            ]
            .into()
        } else {
            with_about
        };

        // Stations/Radio popup — modal on top of the queue view when
        // opened via the "Radio…" button in the queue sidebar.
        let with_stations: Element<'_, GuiMessage> = if self.is_stations_open() {
            iced::widget::stack![
                with_art_popup,
                modal_overlay(
                    super::screens::popups::stations::view(&self.core.state),
                    GuiMessage::CloseStationsPopup,
                ),
            ]
            .into()
        } else {
            with_art_popup
        };

        // Similar popup — overlays on top of the current view when the
        // user picks "Show Similar …" from a context menu. Renders
        // `state.similar` which the LoadSimilar* dispatchers populate.
        let with_similar: Element<'_, GuiMessage> = if self.is_similar_open() {
            iced::widget::stack![
                with_stations,
                modal_overlay(
                    super::screens::popups::similar::view(&self.core.state),
                    GuiMessage::CloseSimilarPopup,
                ),
            ]
            .into()
        } else {
            with_stations
        };

        // Related-artists popup — same pattern as Similar.
        let with_related: Element<'_, GuiMessage> = if self.is_related_open() {
            iced::widget::stack![
                with_similar,
                modal_overlay(
                    super::screens::popups::related::view(&self.core.state),
                    GuiMessage::CloseRelatedPopup,
                ),
            ]
            .into()
        } else {
            with_similar
        };

        // DJ Modes popup — sidebar button on the Now Playing screen.
        let with_dj_modes: Element<'_, GuiMessage> = if self.is_dj_modes_open() {
            iced::widget::stack![
                with_related,
                modal_overlay(
                    super::screens::popups::dj_modes::view(&self.core.state),
                    GuiMessage::CloseDjModesPopup,
                ),
            ]
            .into()
        } else {
            with_related
        };

        // Remix Tools popup — sibling of DJ Modes.
        let with_remix_tools: Element<'_, GuiMessage> = if self.is_remix_tools_open() {
            iced::widget::stack![
                with_dj_modes,
                modal_overlay(
                    super::screens::popups::remix_tools::view(&self.core.state),
                    GuiMessage::CloseRemixToolsPopup,
                ),
            ]
            .into()
        } else {
            with_dj_modes
        };

        // Settings popup — modal wrapper around the Settings screen.
        let with_settings: Element<'_, GuiMessage> = if self.is_settings_open() {
            iced::widget::stack![
                with_remix_tools,
                modal_overlay(
                    super::screens::popups::settings_popup::view(
                        &self.core.state,
                        self.core.config.ui.ui_scale,
                    ),
                    GuiMessage::CloseSettingsPopup,
                ),
            ]
            .into()
        } else {
            with_remix_tools
        };

        // User Guide popup — Help → User Guide.
        let with_user_guide: Element<'_, GuiMessage> = if self.is_user_guide_open() {
            iced::widget::stack![
                with_settings,
                modal_overlay(
                    super::screens::popups::user_guide::view(),
                    GuiMessage::CloseUserGuide,
                ),
            ]
            .into()
        } else {
            with_settings
        };

        // Keyboard Shortcuts popup — Help → Keyboard Shortcuts.
        let with_keyboard_shortcuts: Element<'_, GuiMessage> = if self.is_keyboard_shortcuts_open() {
            iced::widget::stack![
                with_user_guide,
                modal_overlay(
                    super::screens::popups::keyboard_shortcuts::view(),
                    GuiMessage::CloseKeyboardShortcuts,
                ),
            ]
            .into()
        } else {
            with_user_guide
        };

        // App-wide mouse_area tracks cursor position so right-click
        // context menus can anchor to the cursor point. Passes events
        // through to children unchanged.
        let tracked = iced::widget::mouse_area(with_keyboard_shortcuts)
            .on_move(|p| GuiMessage::MouseMoved { x: p.x, y: p.y });

        // Overlay the context menu (if open) on top of everything else.
        if let Some(cm) = self.context_menu.as_ref() {
            let viewport_h = if self.viewport.height > 0 {
                self.viewport.height as f32
            } else {
                (self.core.config.ui.window.height as f32).max(400.0)
            };
            iced::widget::stack![
                tracked,
                super::widgets::context_menu::view(cm, viewport_w_logical, viewport_h),
            ]
            .into()
        } else {
            tracked.into()
        }
    }

    fn subscription(&self) -> Subscription<GuiMessage> {
        // Bridge the shared tokio mpsc (used by all background tasks) into
        // an Iced subscription. Ranked by id; Iced caches on the id so this
        // stream is constructed exactly once per App lifetime.
        let holder = Arc::clone(&self.event_rx);
        let core_events = Subscription::run_with_id(
            "textamp-core-events",
            iced::stream::channel(256, move |mut out| async move {
                let mut rx = match holder.lock().ok().and_then(|mut guard| guard.take()) {
                    Some(rx) => rx,
                    None => return, // already consumed
                };
                while let Some(ev) = rx.recv().await {
                    if out.send(GuiMessage::CoreEvent(ev)).await.is_err() {
                        break;
                    }
                }
            }),
        );

        let keyboard_sub = keyboard::on_key_press(|key, modifiers| {
            super::shortcuts::to_crossterm_key_event(key, modifiers).map(GuiMessage::KeyPress)
        });

        // Resize events drive the responsive scale_factor below: any window
        // size change updates `self.viewport`, which `scale_factor()` reads.
        let resize_sub = iced::window::resize_events().map(|(_id, size)| {
            GuiMessage::WindowResized {
                width: size.width as u32,
                height: size.height as u32,
            }
        });

        // Forward muda menu clicks into Iced messages. The forwarder
        // is installed during the first `update()` tick alongside
        // `init_for_nsapp` — we just drain its tokio receiver here.
        #[cfg(feature = "native-menus")]
        let menu_events = Subscription::run_with_id(
            "textamp-menu-events",
            iced::stream::channel(16, move |mut out| async move {
                // The receiver is taken once: subsequent re-evaluations
                // of `subscription()` would otherwise steal it from the
                // already-running task. `take_forwarder_receiver` returns
                // `None` after the first take, so this branch quietly
                // exits on dupes.
                let Some(mut rx) = super::menu::take_forwarder_receiver() else { return };
                while let Some(msg) = rx.recv().await {
                    tracing::info!("forwarding menu message to update: {msg:?}");
                    if out.send(msg).await.is_err() {
                        break;
                    }
                }
            }),
        );

        // 100 ms tick — drives playback position advancement, visualizer
        // data loading (safety-net in `Event::Tick` handler), marquee /
        // toast expiry, and track-end detection. Matches the TUI's
        // tick_rate in `event_loop.rs`.
        let tick_sub = iced::time::every(std::time::Duration::from_millis(100))
            .map(|_| GuiMessage::Tick);

        // Window-level mouse-button-released listener. The queue
        // drag-and-drop reorder needs to know about *any* left-button
        // release, even ones that land outside a queue row, so the
        // drag state can clear cleanly. Per-row mouse_area on_release
        // wouldn't fire if the drop landed outside the row hitbox.
        //
        // The same subscription forwards `keyboard::Event::ModifiersChanged`
        // events into the app — needed for shift/cmd-click multi-select
        // because iced 0.13's mouse events don't carry modifier state.
        let mouse_and_mods_sub = iced::event::listen_with(|event, _status, _id| match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(GuiMessage::QueueDragEnd)
            }
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(m)) => {
                Some(GuiMessage::ModifiersChanged(m))
            }
            _ => None,
        });

        #[cfg(feature = "native-menus")]
        let subs = vec![core_events, keyboard_sub, resize_sub, tick_sub, mouse_and_mods_sub, menu_events];
        #[cfg(not(feature = "native-menus"))]
        let subs = vec![core_events, keyboard_sub, resize_sub, tick_sub, mouse_and_mods_sub];

        Subscription::batch(subs)
    }

    /// Snap the focused Miller column's scrollable just enough to keep the
    /// selected row in view — only scrolls when the selection is above the
    /// viewport top or below the viewport bottom. Matches desktop list
    /// behaviour (and the TUI's scroll handling).
    ///
    /// The scroll state comes from `scroll_state`, populated by each
    /// scrollable's `on_scroll` callback. On the first render we haven't
    /// seen any scroll events yet, so fall back to the older relative-snap
    /// (selected / total) so the very first arrow-press still moves.
    fn snap_focused_column_into_view(&self) -> Task<GuiMessage> {
        use crate::ui_gui::widgets::miller_column::{row_height_for, scroll_id_for, scroll_offset_for};
        use iced::widget::scrollable::{snap_to, RelativeOffset};

        let nav = match self.core.state.browse_nav() {
            Some(n) => n,
            None => return Task::none(),
        };
        let focused_idx = nav.focused_column;
        let col = match nav.columns.get(focused_idx) {
            Some(c) => c,
            None => return Task::none(),
        };
        let n_items = col.items.len();
        if n_items == 0 {
            return Task::none();
        }
        let selected = col.selected_index;

        let Some(s) = self.scroll_state.get(&focused_idx).copied() else {
            // First time: no scroll info yet — fall back to relative snap
            // so arrow presses still produce motion.
            return snap_to(scroll_id_for(focused_idx), scroll_offset_for(selected, n_items));
        };
        if s.bounds_h <= 0.0 {
            return Task::none();
        }

        // Exact per-row Y positions. With the virtualized miller column
        // we force each row to a fixed pixel height matching
        // `row_height_for`, so the cumulative sum lines up with the
        // content bounds the scrollable reports.
        let show_art = col.artwork_visible;
        let mut sel_top = 0.0_f32;
        for (i, item) in col.items.iter().enumerate() {
            if i == selected { break; }
            sel_top += row_height_for(item, show_art);
        }
        let sel_h = col.items.get(selected).map(|it| row_height_for(it, show_art)).unwrap_or(0.0);
        let sel_bot = sel_top + sel_h;
        let total_h = if s.content_h > 0.0 {
            s.content_h
        } else {
            col.items.iter().map(|it| row_height_for(it, show_art)).sum::<f32>()
        };
        let view_top = s.offset_y;
        let view_bot = s.offset_y + s.bounds_h;

        let new_offset = if sel_top < view_top {
            // Selected is above viewport — scroll up so it sits at the top.
            sel_top
        } else if sel_bot > view_bot {
            // Selected is below viewport — scroll down just enough to bring it in.
            sel_bot - s.bounds_h
        } else {
            // Already visible — no scroll.
            return Task::none();
        };

        // Clamp to valid range.
        let max_off = (total_h - s.bounds_h).max(0.0);
        let new_offset = new_offset.clamp(0.0, max_off);

        // Use `snap_to` (instant, relative offset) instead of `scroll_to`
        // (animated). Holding an arrow key produces key repeats ~30 Hz —
        // iced's animated scroll couldn't keep up and the list visibly
        // stuttered as each new animation interrupted the previous one.
        // Snap-to updates in one frame so held-key scrolling is smooth.
        let rel_y = if max_off <= 0.0 {
            0.0
        } else {
            (new_offset / max_off).clamp(0.0, 1.0)
        };
        snap_to(scroll_id_for(focused_idx), RelativeOffset { x: 0.0, y: rel_y })
    }

    /// Centre every populated Miller column on its selected row. Used
    /// after Actions that re-arrange the column stack via teleport
    /// (Open in Library, Jump to Album, Reveal in Library, …) — the
    /// teleport rebuilds the columns and sets a new `selected_index`
    /// in each, but the scrollable's offset is whatever it was before.
    /// Centring (rather than snapping to the nearest edge) makes the
    /// destination row land mid-viewport so the user can see the
    /// surrounding context, matching the behaviour the user expects:
    /// "the screen needs to look exactly as if I had just navigated to
    /// that point in the columns myself".
    fn center_all_columns_into_view(&self) -> Task<GuiMessage> {
        use crate::ui_gui::widgets::miller_column::{row_height_for, scroll_id_for, scroll_offset_for};
        use iced::widget::scrollable::{snap_to, RelativeOffset};

        let nav = match self.core.state.browse_nav() {
            Some(n) => n,
            None => return Task::none(),
        };

        let mut tasks: Vec<Task<GuiMessage>> = Vec::new();
        for (col_idx, col) in nav.columns.iter().enumerate() {
            let n_items = col.items.len();
            if n_items == 0 { continue; }
            let selected = col.selected_index;

            let task = match self.scroll_state.get(&col_idx).copied() {
                None => {
                    // First time we're rendering this column — no
                    // scroll info yet, fall back to proportional snap
                    // so the selection at least lands roughly in view.
                    snap_to(scroll_id_for(col_idx), scroll_offset_for(selected, n_items))
                }
                Some(s) if s.bounds_h <= 0.0 => continue,
                Some(s) => {
                    let show_art = col.artwork_visible;
                    let mut sel_top = 0.0_f32;
                    for (i, item) in col.items.iter().enumerate() {
                        if i == selected { break; }
                        sel_top += row_height_for(item, show_art);
                    }
                    let sel_h = col.items.get(selected)
                        .map(|it| row_height_for(it, show_art))
                        .unwrap_or(0.0);
                    // Always recompute total_h from the items: after
                    // a category switch, `s.content_h` may still reflect
                    // the previous nav's column and would skew the
                    // centring math. Items are authoritative.
                    let total_h: f32 = col.items.iter()
                        .map(|it| row_height_for(it, show_art))
                        .sum();

                    // Target offset puts the row's centre at the
                    // viewport's centre, then clamp to valid scroll
                    // range so we don't try to scroll past either end
                    // when the selection is near the top or bottom.
                    let target = sel_top + sel_h / 2.0 - s.bounds_h / 2.0;
                    let max_off = (total_h - s.bounds_h).max(0.0);
                    let target = target.clamp(0.0, max_off);
                    let rel_y = if max_off <= 0.0 {
                        0.0
                    } else {
                        (target / max_off).clamp(0.0, 1.0)
                    };
                    snap_to(scroll_id_for(col_idx), RelativeOffset { x: 0.0, y: rel_y })
                }
            };
            tasks.push(task);
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    /// If a dispatched Action flipped `state.view` to `Similar`,
    /// `Related`, or `Settings` (full views on the TUI) and the
    /// matching popup isn't already open, snap the view back to
    /// `prev_view` and open the popup instead. This is what keeps the
    /// GUI's "popup, not screen" promise consistent no matter which
    /// path fired the view change (keyboard, menu bar, context menu,
    /// tab strip).
    fn lift_view_to_popup(&mut self, prev_view: crate::app::state::View) {
        use crate::app::state::View;
        match self.core.state.view {
            View::Similar if !self.is_similar_open() => {
                self.similar_prev_view = (prev_view != View::Similar).then_some(prev_view);
                self.core.state.view = prev_view;
                self.open_similar();
            }
            View::Related if !self.is_related_open() => {
                self.related_prev_view = (prev_view != View::Related).then_some(prev_view);
                self.core.state.view = prev_view;
                self.open_related();
            }
            View::Settings if !self.is_settings_open() => {
                self.core.state.view = prev_view;
                self.open_settings();
            }
            _ => {}
        }
    }

    /// Drive an Action through the shared dispatch router synchronously.
    ///
    /// The dispatchers are `async` but their awaits are overwhelmingly on
    /// bounded `mpsc::Sender::send`, which completes within a handful of
    /// microseconds when the channel has capacity (we size ours to 256).
    /// Real I/O is kicked off via `tokio::spawn` inside each handler, so
    /// `block_on` returns promptly. If a specific dispatcher proves slow
    /// in practice, convert its heavy work to a spawned task.
    fn dispatch_sync(&mut self, action: Action) {
        // If the user is clearing the artwork byte cache, drop our GUI
        // handle cache too — otherwise stale iced handles keep old
        // thumbnails alive in the decoded-image cache.
        let clears_art = matches!(
            &action,
            Action::Settings(crate::app::action::SettingsAction::ClearArtworkCache)
        );
        // Snapshot persistable playback state before dispatch so we can
        // detect changes (volume / mute) and write them to config. The
        // shared `dispatch_playback` doesn't take `&mut Config`, so the
        // GUI front-end is responsible for persisting these.
        let prev_volume = self.core.state.playback.volume;
        let prev_muted = self.core.state.playback.muted;

        let Core { state, client, audio, config, event_tx } = &mut self.core;
        let fut = dispatch::dispatch_action(action, state, client, audio, config, event_tx);
        if let Err(e) = futures::executor::block_on(fut) {
            tracing::error!("dispatch error: {}", e);
        }
        if clears_art {
            super::images::clear_handle_cache();
        }

        // After block_on the destructured `&mut self.core` borrows end —
        // safe to access self.core again. Persist any volume / mute
        // change so it survives a relaunch.
        let new_volume = self.core.state.playback.volume;
        let new_muted = self.core.state.playback.muted;
        if (new_volume - prev_volume).abs() > f32::EPSILON || new_muted != prev_muted {
            self.core.config.playback.default_volume = new_volume;
            if let Err(e) = config::save_config(&self.core.config) {
                tracing::warn!("Failed to save volume: {e}");
            }
        }
    }

    /// Run the periodic-tick work the TUI's event loop does in its `tick`
    /// branch (src/app/event_loop.rs): advance playback position for local
    /// output, detect natural track end, and fan out the `Event::Tick`
    /// follow-up actions (marquee animations, visualizer safety-net loads,
    /// toast/status expiry, periodic progress reporting to Plex).
    ///
    /// Without this, the Now Playing cursor never moves and the visualizer
    /// data never loads when the user is viewing Now Playing.
    /// If the Browse view's currently-focused Miller row is a Track
    /// and we haven't yet cached sonic similars for it (and aren't
    /// currently fetching), kick off `client.get_similar_tracks` as
    /// an iced `Task::perform`. The resolved future arrives back as
    /// `GuiMessage::TrackPaneSimilarLoaded`.
    fn maybe_fetch_track_pane_similar(&mut self) -> Option<Task<GuiMessage>> {
        use crate::app::state::{BrowseItem, View};
        if self.core.state.view != View::Browse {
            return None;
        }
        let track = {
            let nav = self.core.state.browse_nav()?;
            let col = nav.focused()?;
            let item = col.items.get(col.selected_index)?;
            if !matches!(item, BrowseItem::Track { .. }) {
                return None;
            }
            col.tracks.get(col.selected_index)?.clone()
        };
        let key = track.rating_key.clone();
        if self.track_pane_similar.contains_key(&key)
            || self.track_pane_similar_loading.contains(&key)
        {
            return None;
        }
        self.track_pane_similar_loading.insert(key.clone());
        let client = self.core.client.clone();
        let key_for_task = key.clone();
        Some(Task::perform(
            async move {
                client.get_similar_tracks(&key_for_task, 12)
                    .await
                    .unwrap_or_default()
            },
            move |tracks| GuiMessage::TrackPaneSimilarLoaded {
                track_key: key.clone(),
                tracks,
            },
        ))
    }

    /// Handle a keyboard event from the iced subscription. Pulled out
    /// of `update` (which was 1200+ lines) so the dispatcher reads as
    /// a flat table of one-line arms.
    fn handle_key_press(&mut self, key_event: crossterm::event::KeyEvent) -> Task<GuiMessage> {
        use crossterm::event::KeyCode;
        // Escape dismisses GUI-only overlays first (About, dropdown).
        // Only after both are closed does Esc flow through to the
        // shared key_input — matching the usual "modal first" UX.
        if matches!(key_event.code, KeyCode::Esc) {
            if self.art_popup_key.is_some() {
                self.art_popup_key = None;
                return Task::none();
            }
            // Single-popup model: ESC dismisses the active primary
            // popup (if any). The Similar variant additionally
            // restores the captured prev_view.
            if self.primary_popup.is_open() {
                if matches!(self.primary_popup, PrimaryPopup::Similar) {
                    if let Some(v) = self.similar_prev_view.take() {
                        self.core.state.view = v;
                    }
                }
                self.primary_popup = PrimaryPopup::None;
                return Task::none();
            }
            if self.context_menu.is_some() {
                self.context_menu = None;
                return Task::none();
            }
            if self.menu_open.is_some() {
                self.menu_open = None;
                return Task::none();
            }
            // After every overlay is dismissed, ESC clears the
            // transport-bar filter input if it has any text. Mirrors
            // the standard search-box convention.
            if !self.core.state.list_filter.query.is_empty() {
                use crate::app::action::SearchAction;
                self.dispatch_sync(Action::Search(SearchAction::DeactivateListFilter));
                return Task::none();
            }
        }

        // Swallow Ctrl+S — the shared key_input opens a modal sort
        // popup that the GUI doesn't use. Sort options live in the
        // Tools menu instead. (The TUI still opens the popup via its
        // own event loop.)
        if matches!(key_event.code, KeyCode::Char('s'))
            && key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Task::none();
        }

        // F2 opens the Settings popup instead of navigating to a
        // full-screen Settings view.
        if matches!(key_event.code, KeyCode::F(2)) {
            use crate::app::action::SettingsAction;
            self.open_settings();
            self.dispatch_sync(Action::Settings(SettingsAction::RefreshCacheStats));
            return Task::none();
        }

        // F1 opens the User Guide modal.
        if matches!(key_event.code, KeyCode::F(1)) {
            self.open_user_guide();
            return Task::none();
        }

        // UI scale shortcuts (Ctrl++ / Ctrl+- / Ctrl+0). Handled here
        // (not in shared key_input) because UI scale is a GUI-only
        // concern.
        if key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            use crate::config::settings::UI_SCALE_STEP;
            let scale_msg = match key_event.code {
                KeyCode::Char('+') | KeyCode::Char('=') => Some(GuiMessage::AdjustUiScale(UI_SCALE_STEP)),
                KeyCode::Char('-') | KeyCode::Char('_') => Some(GuiMessage::AdjustUiScale(-UI_SCALE_STEP)),
                KeyCode::Char('0') => {
                    let cur = self.core.config.ui.ui_scale;
                    Some(GuiMessage::AdjustUiScale(1.0 - cur))
                }
                _ => None,
            };
            if let Some(msg) = scale_msg {
                return Task::done(msg);
            }
        }

        // Shared dispatcher: same handler the TUI uses to turn a
        // keyboard event into a list of Actions.
        self.core.state.cache_mgmt.last_input_time = std::time::Instant::now();
        let prev_view = self.core.state.view;
        let actions = crate::app::handlers::key_input::handle_key(
            key_event,
            &mut self.core.state,
            &self.core.config,
        );
        for a in actions {
            self.dispatch_sync(a);
        }
        self.lift_view_to_popup(prev_view);
        // Arrow-key / letter-jump navigation changes `selected_index`
        // — snap the scrollable to keep the selected row visible.
        self.snap_focused_column_into_view()
    }

    fn handle_tick(&mut self) {
        use crate::app::event_core::PlaybackEvent;
        use crate::app::state::{OutputTarget, PlayStatus};
        use std::time::Duration as StdDuration;

        const TICK_MS: u64 = 100;
        /// How long the user must be still before the lazy-art gate
        /// reopens. Tuned by the user — short enough that the wait
        /// feels intentional, long enough to absorb a held-down arrow
        /// key without flapping.
        const ART_LOAD_PAUSE: StdDuration = StdDuration::from_millis(1000);

        // Lazy-art gate. While motion has been frequent, every
        // `LoadAlbumArt` returned by the shared dispatcher has been
        // dropped (see `dispatch_system::SystemAction::LoadAlbumArt`).
        // Once the cursor has been still for `ART_LOAD_PAUSE`, clear
        // the flag and re-collect the focused album column's viewport
        // so its art loads in one batch.
        if self.core.state.artwork.suppress_loads
            && self.last_motion_at.elapsed() >= ART_LOAD_PAUSE
        {
            self.core.state.artwork.suppress_loads = false;
            let batch = crate::app::handlers::dispatch_miller::collect_viewport_art(
                &self.core.state,
            );
            if !batch.is_empty() {
                use crate::app::action::SystemAction;
                self.dispatch_sync(Action::System(SystemAction::LoadAlbumArt(batch)));
            }
        }

        // Audio self-heal. Windows can refuse to expose a default
        // output device for several seconds after the previous holder
        // exited (a fresh kill+relaunch cycle in particular). The
        // initial 12 s startup retry inside `App::new` covers the
        // common case, but not pathological ones — keep trying in the
        // background, every ~2 s, while audio is still unavailable.
        // Without this the user has to close and reopen the app to
        // recover, which we explicitly want to avoid.
        if !self.core.state.audio_available {
            self.audio_retry_ticks_left = self.audio_retry_ticks_left.saturating_sub(1);
            if self.audio_retry_ticks_left == 0 {
                match self.core.audio.try_attach_backend() {
                    Ok(true) => {
                        self.core.state.audio_available = true;
                        self.core.state.notifications.last_error = None;
                        self.core.state.set_status(
                            "Audio device connected.".to_string(),
                        );
                    }
                    Ok(false) | Err(_) => {
                        // Keep retrying ~every 2 seconds.
                        self.audio_retry_ticks_left = 20;
                    }
                }
            }
            // Re-grab the tap if a backend just attached.
            if self.vectorscope_tap.is_none() {
                self.vectorscope_tap = self.core.audio.sample_tap();
            }
        }

        // Pull fresh stereo samples from the rodio tap into the
        // vectorscope buffer. We always drain (whether or not the
        // vectorscope tab is visible) so re-opening the tab shows
        // current audio rather than a stale frame, and so the buffer
        // stays bounded — TapSource is allowed to fill SAMPLE_TAP_CAP
        // samples between drains. The cost is a brief Mutex lock
        // every 100 ms; the lock is held for ~µs.
        if let Some(tap) = self.vectorscope_tap.as_ref() {
            if let Ok(mut buf) = tap.lock() {
                if !buf.is_empty() {
                    let pairs: Vec<(f32, f32)> = buf.drain(..).collect();
                    self.vectorscope.push_samples(&pairs);
                }
            }
        }

        // 1. Advance playback position on local output when playing.
        let is_local = matches!(self.core.state.remote.output_target, OutputTarget::Local);
        if self.core.state.playback.status == PlayStatus::Playing && is_local {
            self.core.state.playback.position_ms =
                self.core.state.playback.position_ms.saturating_add(TICK_MS);

            // Detect natural track end: sink drained, ≥1 s of playback
            // elapsed (avoids cold-start false positives), and we played
            // at least ~90 % of the known duration.
            let started_long_enough = self.core.state.playback.playback_started_at
                .map(|t| t.elapsed() >= StdDuration::from_secs(1))
                .unwrap_or(false);
            if started_long_enough && self.core.audio.is_finished() {
                let expected = self.core.state.playback.duration_ms;
                let actual = self.core.audio.position()
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(self.core.state.playback.position_ms);
                let completed = expected == 0
                    || actual >= expected * 90 / 100
                    || (expected > 5_000 && actual >= expected.saturating_sub(5_000));

                let evt = if completed {
                    PlaybackEvent::TrackEnded
                } else {
                    tracing::warn!(
                        "Premature track end detected in GUI tick: played {}ms of {}ms expected",
                        actual,
                        expected,
                    );
                    PlaybackEvent::PlaybackError("Track ended prematurely".to_string())
                };
                let tx = self.core.event_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(Event::Playback(evt)).await;
                });
                // Freeze further ticks from re-firing TrackEnded until the
                // Action router updates status. The TrackEnded handler
                // will set the final status and kick off the next track.
                self.core.state.playback.status = PlayStatus::Paused;
            }
        }

        // 2. Fan out Event::Tick follow-up actions (marquee, toast expiry,
        //    visualizer safety-net, periodic progress report).
        let actions = crate::app::handlers::events::handle_app_event(
            Event::Tick,
            &mut self.core.state,
            &mut self.core.client,
            &self.core.event_tx,
        );
        for a in actions {
            self.dispatch_sync(a);
        }
    }
}

/// Wrap a popup element in a full-screen modal overlay. The overlay
/// dims the backdrop and — critically — absorbs mouse clicks so they
/// don't pass through to the view underneath. A click on empty space
/// outside the popup fires `close_msg`; clicks on widgets inside the
/// popup are routed normally (buttons consume the event before the
/// wrapping `mouse_area` sees it).
fn modal_overlay<'a>(
    content: Element<'a, GuiMessage>,
    close_msg: GuiMessage,
) -> Element<'a, GuiMessage> {
    use iced::widget::{container, mouse_area, stack};
    // Two-layer modal: the dim full-screen `mouse_area` catches any
    // click that lands outside the centred popup and dismisses it.
    // The popup itself sits ABOVE the dim layer so its own buttons
    // capture the click first (Iced's `Stack::on_event` polls
    // top-down and short-circuits on Captured).
    //
    // Wrapping the popup INSIDE the dim mouse_area — the previous
    // implementation — caused the dim's `on_press` to ALSO fire on
    // every click that landed on a popup button, dismissing the
    // popup mid-interaction. Separate sibling layers fix it.
    let dim: Element<'a, GuiMessage> = container(iced::widget::Space::new(Length::Fill, Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 })),
            ..container::Style::default()
        })
        .into();
    let dismisser: Element<'a, GuiMessage> = mouse_area(dim)
        .on_press(close_msg.clone())
        .on_release(GuiMessage::Noop)
        .on_right_press(close_msg)
        .on_right_release(GuiMessage::Noop)
        .on_middle_press(GuiMessage::Noop)
        .on_middle_release(GuiMessage::Noop)
        .into();
    let centered_popup: Element<'a, GuiMessage> = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();
    stack![dismisser, centered_popup].into()
}

fn state_browse_category_is_folders(state: &AppState) -> bool {
    matches!(state.browse_category, crate::app::state::BrowseCategory::Folders)
}

/// Build the search query for an external-music-service search of the
/// right-clicked row. Mirrors the formats produced by
/// `build_external_search_query` (palette path) — "Artist - Album"
/// for tracks/albums, just the artist name for artist rows — but
/// targets the right-clicked item directly rather than focused state.
/// Returns `None` for non-music rows (genres, group headers, etc.) so
/// the caller can suppress the entries entirely.
fn external_search_query_for_item(
    item: &crate::app::state::BrowseItem,
    col: &crate::app::state::BrowseColumn,
    item_index: usize,
) -> Option<String> {
    use crate::app::state::BrowseItem;
    match item {
        BrowseItem::Track { artist_name, album_name, title, .. } => {
            // Prefer the parallel `tracks` array (full Track has
            // resolved metadata) over the BrowseItem's optional fields.
            if let Some(t) = col.tracks.get(item_index) {
                let artist = t.artist_name();
                let album = t.album_name();
                if !artist.is_empty() && !album.is_empty() {
                    return Some(format!("{} - {}", artist, album));
                }
            }
            match (artist_name.as_deref(), album_name.as_deref()) {
                (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() =>
                    Some(format!("{} - {}", a, b)),
                _ => Some(title.clone()),
            }
        }
        BrowseItem::Album { title, artist, .. } => {
            if !artist.is_empty() {
                Some(format!("{} - {}", artist, title))
            } else {
                Some(title.clone())
            }
        }
        BrowseItem::Artist { title, .. } => Some(title.clone()),
        BrowseItem::Playlist { title, .. } => Some(title.clone()),
        BrowseItem::AllTracks { artist_name, .. } if !artist_name.is_empty() =>
            Some(artist_name.clone()),
        _ => None,
    }
}

/// Build the right-click context-menu entries for a Miller-column row.
/// Returns `None` when the click lands on an empty row or an item type
/// that has no useful per-item actions (the menu simply does not open).
///
/// Entries match the TUI's keyboard shortcuts for the same item kind —
/// the menu is a discoverability aid, not a new surface for functions.
fn build_miller_context_menu(
    state: &AppState,
    column_index: usize,
    item_index: usize,
    mouse_pos: (f32, f32),
) -> Option<super::widgets::context_menu::ContextMenuState> {
    use crate::app::action::{DataAction, MillerAction, NavigationAction, QueueAction, RadioAction, SearchAction, SystemAction};
    use crate::app::state::{BrowseItem, View};
    use super::widgets::context_menu::{ContextMenuState, Entry};

    let nav = state.browse_nav()?;
    let col = nav.columns.get(column_index)?;
    let item = col.items.get(item_index)?;

    let mut entries: Vec<Entry> = Vec::new();

    match item {
        BrowseItem::Track { .. } => {
            // Pick the right play action for the active browse category.
            // Each category has its own nav state and matching dispatcher.
            let play_action = |single: bool| -> Action {
                use crate::app::state::BrowseCategory;
                match state.browse_category {
                    BrowseCategory::Genres => Action::Miller(MillerAction::PlayGenreTrackFromMiller {
                        column_index, track_index: item_index, single_track: single,
                    }),
                    BrowseCategory::Playlists => Action::Miller(MillerAction::PlayPlaylistTrackFromMiller {
                        column_index, track_index: item_index, single_track: single,
                    }),
                    _ => Action::Miller(MillerAction::PlayTrackFromMiller {
                        column_index, track_index: item_index, single_track: single,
                    }),
                }
            };
            // Multi-selection: when the right-clicked row is part of
            // an active selection set on this column, the menu acts on
            // every selected track rather than the single right-clicked
            // row. We materialise the Tracks here (the selected_set is
            // index-based; the parallel `col.tracks` carries the full
            // payloads). "Play track" is always the singular form
            // because it sets the cursor / starts playback at one
            // point, but Add-to-queue and Play-next become bulk.
            let multi_active = !col.selected_set.is_empty()
                && col.selected_set.contains(&item_index);
            let multi_tracks: Vec<crate::plex::models::Track> = if multi_active {
                col.selected_set.iter()
                    .filter_map(|&i| col.tracks.get(i).cloned())
                    .collect()
            } else {
                Vec::new()
            };
            let multi_count = multi_tracks.len();

            // Singular vs plural — when a multi-selection is active,
            // every label that names the unit-of-action says "tracks"
            // so it's obvious the menu acts on the whole selection.
            let unit = if multi_active && multi_count > 1 { "tracks" } else { "track" };
            entries.push(Entry::Entry {
                label: format!("Play {unit}"),
                actions: vec![play_action(true)],
            });
            entries.push(Entry::Entry {
                label: format!("Play {unit} and following"),
                actions: vec![play_action(false)],
            });
            if multi_active && multi_count > 1 {
                let bulk_enqueue: Vec<Action> = multi_tracks.iter()
                    .cloned()
                    .map(|t| Action::Queue(QueueAction::EnqueueTrack(t)))
                    .collect();
                entries.push(Entry::Entry {
                    label: format!("Add {multi_count} tracks to end of queue"),
                    actions: bulk_enqueue,
                });
                entries.push(Entry::Entry {
                    label: format!("Play {multi_count} tracks next in queue"),
                    actions: vec![Action::Queue(QueueAction::EnqueueTracksNext(multi_tracks.clone()))],
                });
            } else {
                entries.push(Entry::Entry {
                    label: "Add to end of queue".to_string(),
                    actions: vec![Action::Queue(QueueAction::EnqueueSelection)],
                });
                entries.push(Entry::Entry {
                    label: "Play next in queue".to_string(),
                    actions: vec![Action::Queue(QueueAction::EnqueueSelectionNext)],
                });
            }
            entries.push(Entry::Sep);

            if let BrowseItem::Track { key, title, .. } = item {
                entries.push(Entry::Custom {
                    label: "Show Similar Tracks".to_string(),
                    message: super::message::GuiMessage::ShowSimilarPopup(vec![
                        Action::Data(DataAction::LoadSimilarTracks {
                            rating_key: key.clone(),
                            title: title.clone(),
                        }),
                    ]),
                });
            }
            // Sonic Adventure from a track row: pre-fill the start
            // track from the column's parallel `tracks` array so the
            // launcher opens at the count step, not the search step.
            // Falls back to the unseeded launcher if the row has no
            // backing Track (shouldn't happen for Track rows, but the
            // tracks array is technically separate).
            let adventure_action: Action = match col.tracks.get(item_index).cloned() {
                Some(t) => Action::Search(SearchAction::OpenAdventureLauncherWithStart {
                    start_track: Box::new(t),
                }),
                None => Action::Search(SearchAction::OpenAdventureLauncher),
            };
            entries.push(Entry::Entry {
                label: "Sonic Adventure\u{2026}".to_string(),
                actions: vec![adventure_action],
            });
            entries.push(Entry::Sep);

            entries.push(Entry::Entry {
                label: "Go to Now Playing".to_string(),
                actions: vec![
                    Action::Navigation(NavigationAction::SetView(View::NowPlaying)),
                    Action::System(SystemAction::LoadWaveform),
                ],
            });
            // "Show Artist Bio" — pulls the artist key + name off the
            // full Track stored in the column's `tracks` parallel array.
            // Always available on Track rows so the bio is one click
            // away whether the user is in Library, Playlists, search,
            // or anywhere else with track rows.
            if let Some(t) = col.tracks.get(item_index) {
                if let Some(akey) = t.grandparent_rating_key.clone() {
                    let aname = t.artist_name().to_string();
                    if !aname.is_empty() {
                        entries.push(Entry::Entry {
                            label: "Show Artist Bio".to_string(),
                            actions: vec![Action::Search(SearchAction::ShowArtistBio {
                                artist_key: akey,
                                artist_name: aname,
                            })],
                        });
                    }
                }
            }
            // "Open in Library": navigate to the artist + album for
            // this track. Pulls album_key + artist_key off the full
            // Track stored in the column's `tracks` parallel array.
            // Suppressed when the user is already in the Library
            // category — the entry would just take them where they
            // already are.
            let in_library = state.browse_category == crate::app::state::BrowseCategory::Library;
            if !in_library {
                if let Some(open) = open_in_library_for_track(col, item_index) {
                    entries.push(Entry::Sep);
                    entries.push(Entry::Entry { label: "Open in Library".to_string(), actions: vec![open] });
                }
            }
        }
        BrowseItem::Album { key, title, .. } => {
            entries.push(Entry::Entry {
                label: "Play Album".to_string(),
                actions: vec![Action::Queue(QueueAction::PlayAlbumNow {
                    rating_key: key.clone(),
                    title: title.clone(),
                })],
            });
            entries.push(Entry::Entry {
                label: "Add album to end of queue".to_string(),
                actions: vec![Action::Queue(QueueAction::EnqueueAlbum {
                    rating_key: key.clone(),
                    title: title.clone(),
                })],
            });
            entries.push(Entry::Entry {
                label: "Play album next in queue".to_string(),
                actions: vec![Action::Queue(QueueAction::EnqueueAlbumNext {
                    rating_key: key.clone(),
                    title: title.clone(),
                })],
            });
            entries.push(Entry::Sep);
            entries.push(Entry::Custom {
                label: "Show Similar Albums".to_string(),
                message: super::message::GuiMessage::ShowSimilarPopup(vec![
                    Action::Data(DataAction::LoadSimilarAlbums {
                        rating_key: key.clone(),
                        title: title.clone(),
                    }),
                ]),
            });
            entries.push(Entry::Entry {
                label: "Sonic Adventure\u{2026}".to_string(),
                actions: vec![Action::Search(SearchAction::OpenAdventureLauncher)],
            });
            // "Open in Library": find the artist by name and drill in.
            // Album rows don't carry an artist_key, so we look it up
            // in `state.library.artists` by exact name match.
            if let Some(open) = open_in_library_for_album(state, key, title) {
                entries.push(Entry::Sep);
                entries.push(Entry::Entry { label: "Open in Library".to_string(), actions: vec![open] });
            }
        }
        BrowseItem::Artist { key, title, .. } => {
            // No "Play Artist Tracks" / "Enqueue Artist Tracks" here —
            // both are one drill away (pinned rows inside the artist's
            // column). Keeping the context menu to the things that
            // *aren't* obvious by drilling.
            entries.push(Entry::Entry {
                label: "Artist Radio".to_string(),
                actions: vec![Action::Radio(RadioAction::StartPlexRadio {
                    key: key.clone(),
                    title: title.clone(),
                })],
            });
            entries.push(Entry::Custom {
                label: "Similar Artists".to_string(),
                message: super::message::GuiMessage::ShowSimilarPopup(vec![
                    Action::Data(DataAction::LoadSimilarArtists {
                        artist_key: key.clone(),
                        title: title.clone(),
                    }),
                ]),
            });
            entries.push(Entry::Custom {
                label: "Related Artists".to_string(),
                message: super::message::GuiMessage::ShowRelatedPopup(vec![
                    Action::Data(DataAction::LoadRelated {
                        artist_key: key.clone(),
                        title: title.clone(),
                    }),
                ]),
            });
            entries.push(Entry::Sep);
            entries.push(Entry::Entry {
                label: "Artist Bio".to_string(),
                actions: vec![Action::Search(SearchAction::ShowArtistBio {
                    artist_key: key.clone(),
                    artist_name: title.clone(),
                })],
            });
        }
        BrowseItem::Playlist { key, title, .. } => {
            entries.push(Entry::Entry {
                label: "Play Playlist".to_string(),
                actions: vec![Action::Queue(QueueAction::PlayPlaylistNow {
                    playlist_key: key.clone(),
                    title: title.clone(),
                })],
            });
        }
        _ => {
            // No contextual actions for Genre / GenreCategory / AllArtists /
            // AllTracks / Compilations / ArtistRadio — they're either
            // single-action items already (handled by left click) or group
            // headers.
            return None;
        }
    }

    // External search: parity with the command palette. The query is
    // computed from the right-clicked row directly (NOT from focused
    // state — right-click does not move focus) so the URL targets the
    // row the user actually clicked. Each entry is gated by the
    // matching "Search ⟨service⟩" toggle in Settings; disabled
    // services are hidden entirely.
    if let Some(query) = external_search_query_for_item(item, col, item_index) {
        let any_enabled = state.external_search.apple_music
            || state.external_search.spotify
            || state.external_search.youtube;
        if any_enabled {
            entries.push(Entry::Sep);
        }
        if state.external_search.apple_music {
            entries.push(Entry::Entry {
                label: format!("Search Apple Music for \u{201c}{}\u{201d}", query),
                actions: vec![Action::System(SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::AppleMusic,
                    query: Some(query.clone()),
                })],
            });
        }
        if state.external_search.spotify {
            entries.push(Entry::Entry {
                label: format!("Search Spotify for \u{201c}{}\u{201d}", query),
                actions: vec![Action::System(SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::Spotify,
                    query: Some(query.clone()),
                })],
            });
        }
        if state.external_search.youtube {
            entries.push(Entry::Entry {
                label: format!("Search YouTube for \u{201c}{}\u{201d}", query),
                actions: vec![Action::System(SystemAction::OpenExternalSearch {
                    target: crate::services::external_search::SearchTarget::YouTube,
                    query: Some(query),
                })],
            });
        }
    }

    if entries.is_empty() {
        return None;
    }

    Some(ContextMenuState {
        x: mouse_pos.0,
        y: mouse_pos.1,
        entries,
    })
}

/// Right-click menu for a queue row: play / play next / add / remove.
fn build_queue_context_menu(
    state: &AppState,
    row_index: usize,
    mouse_pos: (f32, f32),
) -> Option<super::widgets::context_menu::ContextMenuState> {
    use crate::app::action::{DataAction, QueueAction, RadioAction, SearchAction};
    use crate::app::state::PlaybackMode;
    use super::widgets::context_menu::{ContextMenuState, Entry};

    let tracks = match state.playback_mode {
        PlaybackMode::Radio => &state.radio.tracks,
        _ => &state.queue.tracks,
    };
    let len = tracks.len();
    if row_index >= len { return None; }
    let track = tracks.get(row_index).cloned();

    let mut entries: Vec<Entry> = Vec::new();

    let play_action = match state.playback_mode {
        PlaybackMode::Radio => Action::Radio(RadioAction::JumpToRadioTrack(row_index)),
        _ => Action::Queue(QueueAction::PlayTrackFromCategory(row_index)),
    };
    entries.push(Entry::Entry { label: "Play".to_string(), actions: vec![play_action] });

    // Add-to-queue entries make sense only in Queue mode (Radio
    // auto-streams its own tracks). Move / delete works in both
    // modes — the dispatcher handles the Radio → Queue promotion
    // implicitly when the user starts editing the list.
    if !matches!(state.playback_mode, PlaybackMode::Radio) {
        entries.push(Entry::Entry {
            label: "Play next in queue".to_string(),
            actions: vec![Action::Queue(QueueAction::EnqueueSelectionNext)],
        });
        entries.push(Entry::Entry {
            label: "Add to end of queue".to_string(),
            actions: vec![Action::Queue(QueueAction::EnqueueSelection)],
        });
    }
    entries.push(Entry::Sep);
    // Move up / down / delete: when a multi-selection is active and
    // the right-clicked row is part of it, the menu operates on the
    // whole selection (shared `MoveSelectedTracksUp/Down` and
    // `RemoveSelectedFromQueue` handlers). Otherwise it falls back to
    // the single-row variants targeting just the right-clicked row.
    let multi_active = !state.queue.selected.is_empty()
        && state.queue.selected.contains(&row_index);
    let multi_count = state.queue.selected.len();

    if multi_active && multi_count > 1 {
        entries.push(Entry::Entry {
            label: format!("Move {multi_count} selected up"),
            actions: vec![Action::Queue(QueueAction::MoveSelectedTracksUp)],
        });
        entries.push(Entry::Entry {
            label: format!("Move {multi_count} selected down"),
            actions: vec![Action::Queue(QueueAction::MoveSelectedTracksDown)],
        });
        entries.push(Entry::Entry {
            label: format!("Delete {multi_count} selected from queue"),
            actions: vec![Action::Queue(QueueAction::RemoveSelectedFromQueue)],
        });
    } else {
        if row_index > 0 {
            entries.push(Entry::Custom {
                label: "Move up".to_string(),
                message: GuiMessage::MoveQueueRowUp(row_index),
            });
        }
        if row_index + 1 < len {
            entries.push(Entry::Custom {
                label: "Move down".to_string(),
                message: GuiMessage::MoveQueueRowDown(row_index),
            });
        }
        entries.push(Entry::Entry {
            label: "Delete from queue".to_string(),
            actions: vec![Action::Queue(QueueAction::RemoveFromQueue(row_index))],
        });
    }

    // Similar / artist-context entries: every queue track row gets
    // the same sonic-similarity affordances Browse offers, so the
    // user never has to round-trip through Library to find them.
    // "Show Similar Tracks" loads tracks that are sonically similar
    // (Plex `/similar` endpoint); "Show Similar Albums" pivots to the
    // parent album. "Show Artist Bio" / "Open in Library" stay at the
    // bottom as before.
    if let Some(t) = track {
        let track_key = t.rating_key.clone();
        let track_label = format!("{} - {}", t.artist_name(), t.title);
        let album_key = t.parent_rating_key.clone();
        let album_label = t.album_name().to_string();

        entries.push(Entry::Sep);
        entries.push(Entry::Custom {
            label: "Show Similar Tracks".to_string(),
            message: super::message::GuiMessage::ShowSimilarPopup(vec![
                Action::Data(DataAction::LoadSimilarTracks {
                    rating_key: track_key,
                    title: track_label,
                }),
            ]),
        });
        if let (Some(ak), false) = (album_key, album_label.is_empty()) {
            entries.push(Entry::Custom {
                label: "Show Similar Albums".to_string(),
                message: super::message::GuiMessage::ShowSimilarPopup(vec![
                    Action::Data(DataAction::LoadSimilarAlbums {
                        rating_key: ak,
                        title: album_label,
                    }),
                ]),
            });
        }

        if let Some(akey) = t.grandparent_rating_key.clone() {
            let aname = t.artist_name().to_string();
            if !aname.is_empty() {
                entries.push(Entry::Custom {
                    label: "Related Artists".to_string(),
                    message: super::message::GuiMessage::ShowRelatedPopup(vec![
                        Action::Data(DataAction::LoadRelated {
                            artist_key: akey.clone(),
                            title: aname.clone(),
                        }),
                    ]),
                });
                entries.push(Entry::Sep);
                entries.push(Entry::Entry {
                    label: "Show Artist Bio".to_string(),
                    actions: vec![Action::Search(SearchAction::ShowArtistBio {
                        artist_key: akey.clone(),
                        artist_name: aname.clone(),
                    })],
                });
                if let Some(open) = open_in_library_for_track_obj(&t) {
                    entries.push(Entry::Entry {
                        label: "Open in Library".to_string(),
                        actions: vec![open],
                    });
                }
            }
        }
    }

    Some(ContextMenuState {
        x: mouse_pos.0,
        y: mouse_pos.1,
        entries,
    })
}

/// Build the right-click context menu for a row that carries a
/// full `Track` payload but isn't anchored to a Miller column or
/// queue position — e.g. the "Sonically Similar" rows in the Browse
/// track-details pane. Mirrors the standard track menu the rest of
/// the app uses (queue / Miller column track row): Play, Play next /
/// Add to queue, Show Similar Tracks, Show Similar Albums, Related
/// Artists, Show Artist Bio, Open in Library, Sonic Adventure.
fn build_track_context_menu(
    _state: &AppState,
    track: crate::plex::models::Track,
    mouse_pos: (f32, f32),
) -> Option<super::widgets::context_menu::ContextMenuState> {
    use crate::app::action::{DataAction, QueueAction, SearchAction};
    use super::widgets::context_menu::{ContextMenuState, Entry};

    let mut entries: Vec<Entry> = Vec::new();
    let track_key = track.rating_key.clone();
    let track_label = format!("{} - {}", track.artist_name(), track.title);
    let album_key = track.parent_rating_key.clone();
    let album_label = track.parent_title.clone();
    let artist_key = track.grandparent_rating_key.clone();
    let artist_name = track.artist_name().to_string();

    entries.push(Entry::Entry {
        label: "Play track".to_string(),
        actions: vec![Action::Queue(QueueAction::PlayTrack(track.clone()))],
    });
    entries.push(Entry::Entry {
        label: "Play next in queue".to_string(),
        actions: vec![Action::Queue(QueueAction::EnqueueTracksNext(vec![track.clone()]))],
    });
    entries.push(Entry::Entry {
        label: "Add to end of queue".to_string(),
        actions: vec![Action::Queue(QueueAction::EnqueueTrack(track.clone()))],
    });
    entries.push(Entry::Sep);

    entries.push(Entry::Custom {
        label: "Show Similar Tracks".to_string(),
        message: super::message::GuiMessage::ShowSimilarPopup(vec![
            Action::Data(DataAction::LoadSimilarTracks {
                rating_key: track_key.clone(),
                title: track_label.clone(),
            }),
        ]),
    });
    if let (Some(ak), Some(al)) = (album_key.clone(), album_label.clone()) {
        if !al.is_empty() {
            entries.push(Entry::Custom {
                label: "Show Similar Albums".to_string(),
                message: super::message::GuiMessage::ShowSimilarPopup(vec![
                    Action::Data(DataAction::LoadSimilarAlbums {
                        rating_key: ak,
                        title: al,
                    }),
                ]),
            });
        }
    }
    if let Some(ak) = artist_key.clone() {
        if !artist_name.is_empty() {
            entries.push(Entry::Custom {
                label: "Related Artists".to_string(),
                message: super::message::GuiMessage::ShowRelatedPopup(vec![
                    Action::Data(DataAction::LoadRelated {
                        artist_key: ak,
                        title: artist_name.clone(),
                    }),
                ]),
            });
        }
    }
    entries.push(Entry::Entry {
        label: "Sonic Adventure\u{2026}".to_string(),
        actions: vec![Action::Search(SearchAction::OpenAdventureLauncherWithStart {
            start_track: Box::new(track.clone()),
        })],
    });

    if let Some(ak) = artist_key {
        if !artist_name.is_empty() {
            entries.push(Entry::Sep);
            entries.push(Entry::Entry {
                label: "Show Artist Bio".to_string(),
                actions: vec![Action::Search(SearchAction::ShowArtistBio {
                    artist_key: ak,
                    artist_name: artist_name.clone(),
                })],
            });
            if let Some(open) = open_in_library_for_track_obj(&track) {
                entries.push(Entry::Entry {
                    label: "Open in Library".to_string(),
                    actions: vec![open],
                });
            }
        }
    }

    if entries.is_empty() {
        return None;
    }
    Some(ContextMenuState {
        x: mouse_pos.0,
        y: mouse_pos.1,
        entries,
    })
}

/// Build the right-click context menu for a Folders-category track
/// row. The Folders nav stores `FolderItem` values rather than the
/// usual `BrowseItem::Track`, so we can't reuse
/// `build_miller_context_menu` directly — but the menu entries
/// match: Play, Show Artist Bio, Open in Library. Artist name is
/// looked up from the cached library roster by the track's
/// `grandparent_rating_key`.
fn build_folder_context_menu(
    state: &AppState,
    row_index: usize,
    mouse_pos: (f32, f32),
) -> Option<super::widgets::context_menu::ContextMenuState> {
    use crate::app::action::{BrowseAction, FolderAction, SearchAction};
    use crate::plex::models::FolderItemType;
    use super::widgets::context_menu::{ContextMenuState, Entry};

    let folder_state = state.folder_state.as_ref()?;
    let col = folder_state.focused()?;
    let item = col.items.get(row_index)?;
    if !matches!(item.item_type, FolderItemType::Track) {
        return None;
    }

    let mut entries: Vec<Entry> = Vec::new();
    entries.push(Entry::Custom {
        label: "Play".to_string(),
        message: GuiMessage::Action(Action::Folders(FolderAction::PlayFolderTrack { track_index: row_index })),
    });

    // Look up the artist name from the cached roster — FolderItem
    // doesn't carry artist titles. Falls back to skipping the
    // artist-dependent entries when neither nav has the rating key,
    // rather than showing a menu item that opens an empty Library.
    let artist_key = item.grandparent_rating_key.clone();
    let artist_name = artist_key.as_ref().and_then(|k| {
        state.library.artists.iter()
            .find(|a| &a.rating_key == k)
            .or_else(|| state.library.track_artists.iter().find(|a| &a.rating_key == k))
            .map(|a| a.title.clone())
    });

    if let (Some(akey), Some(aname)) = (artist_key.clone(), artist_name.clone()) {
        if !aname.is_empty() {
            entries.push(Entry::Sep);
            entries.push(Entry::Entry {
                label: "Show Artist Bio".to_string(),
                actions: vec![Action::Search(SearchAction::ShowArtistBio {
                    artist_key: akey.clone(),
                    artist_name: aname.clone(),
                })],
            });
            // Open in Library: needs both album_key (parent) and
            // artist_key (grandparent). Album title isn't on the
            // FolderItem either; the dispatcher fills it in from
            // the library cache after the drill resolves.
            if let Some(album_key) = item.parent_rating_key.clone() {
                entries.push(Entry::Entry {
                    label: "Open in Library".to_string(),
                    actions: vec![Action::Browse(BrowseAction::OpenInLibrary {
                        artist_key: akey,
                        artist_name: aname,
                        album_key: Some(album_key),
                        album_title: None,
                    })],
                });
            }
        }
    }

    Some(ContextMenuState {
        x: mouse_pos.0,
        y: mouse_pos.1,
        entries,
    })
}

/// Build an `OpenInLibrary` action from a full Track value (no column
/// context needed). Used by the queue context menu, where rows are
/// raw Tracks rather than `BrowseColumn` items.
fn open_in_library_for_track_obj(track: &crate::plex::models::Track) -> Option<Action> {
    use crate::app::action::BrowseAction;
    let artist_key = track.grandparent_rating_key.clone()?;
    let album_key = track.parent_rating_key.clone();
    Some(Action::Browse(BrowseAction::OpenInLibrary {
        artist_key,
        artist_name: track.artist_name().to_string(),
        album_key,
        album_title: Some(track.album_name().to_string()),
    }))
}

/// Resolve a Miller-column row click to the Actions that mirror the TUI's
/// mouse handler for the same click:
///
/// - If the click lands on a column that isn't focused, just move focus
///   and select that row (pure state mutation — no follow-up).
/// - If the click lands on the focused column:
///   - non-activating click: update selection in place (no drill).
///   - activating click (already-selected row clicked again): emit the
/// Build an `OpenInLibrary` action for a clicked Track row, using the
/// full Track stored in the column's `tracks` parallel array (which
/// carries `parent_rating_key` = album, `grandparent_rating_key` =
/// artist). Returns `None` if either key is missing.
fn open_in_library_for_track(
    col: &crate::app::state::BrowseColumn,
    item_index: usize,
) -> Option<Action> {
    use crate::app::action::BrowseAction;
    let track = col.tracks.get(item_index)?;
    let artist_key = track.grandparent_rating_key.clone()?;
    let album_key = track.parent_rating_key.clone();
    Some(Action::Browse(BrowseAction::OpenInLibrary {
        artist_key,
        artist_name: track.artist_name().to_string(),
        album_key,
        album_title: Some(track.album_name().to_string()),
    }))
}

/// Build an `OpenInLibrary` action for a clicked Album row. Album rows
/// in non-Library categories don't carry an artist key — look the
/// artist up by exact name match in `state.library.artists`. Returns
/// `None` if the artist isn't found in the library cache.
fn open_in_library_for_album(
    state: &AppState,
    album_key: &str,
    album_title: &str,
) -> Option<Action> {
    use crate::app::action::BrowseAction;
    use crate::app::state::BrowseItem;
    // Look in the focused column for an Album with a matching key
    // and grab the artist field; that gives us a name to search by.
    let nav = state.browse_nav()?;
    let artist_name = nav.focused().and_then(|c| {
        c.items.iter().find_map(|it| match it {
            BrowseItem::Album { key, artist, .. } if key == album_key => Some(artist.clone()),
            _ => None,
        })
    }).unwrap_or_default();
    if artist_name.is_empty() { return None; }
    let artist = state.library.artists.iter().find(|a| a.title == artist_name)?;
    Some(Action::Browse(BrowseAction::OpenInLibrary {
        artist_key: artist.rating_key.clone(),
        artist_name: artist.title.clone(),
        album_key: Some(album_key.to_string()),
        album_title: Some(album_title.to_string()),
    }))
}

/// Local re-export of the shared `helpers::drill_grouped_album` so the
/// GUI's `miller_click_actions` doesn't have to re-import the long path.
fn helpers_for_grouped_album(
    col: &crate::app::state::BrowseColumn,
    album_idx: usize,
) -> Option<crate::app::state::BrowseColumn> {
    crate::app::handlers::helpers::drill_grouped_album(col, album_idx)
}

/// Find the row index in the root browse column whose sort key matches
/// the requested character class. Pure lookup — does NOT mutate state.
///
/// - `'a'..='z'`: first item whose sort key starts with that letter.
/// - `'0'`: first item whose sort key starts with a digit.
/// - `'%'`: first item whose sort key starts with a non-alphanumeric
///   character (a "symbol").
///
/// Returns `None` for the Folders category (which uses a different nav)
/// or when the focused nav has no columns / no matching items.
fn alphabet_target_index(state: &AppState, ch: char) -> Option<usize> {
    use crate::app::handlers::helpers::sort_key;
    use crate::app::state::BrowseCategory;

    if state.browse_category == BrowseCategory::Folders {
        return None;
    }
    let target = ch.to_ascii_lowercase();
    let pred: Box<dyn Fn(&str) -> bool> = match target {
        '0' => Box::new(|t: &str| sort_key(t).chars().next().map_or(false, |c| c.is_ascii_digit())),
        '%' => Box::new(|t: &str| sort_key(t).chars().next().map_or(false, |c| !c.is_ascii_alphanumeric())),
        c if c.is_ascii_alphabetic() => Box::new(move |t: &str| {
            sort_key(t).chars().next().map_or(false, |first| first.to_ascii_lowercase() == c)
        }),
        _ => return None,
    };

    let nav = state.browse_nav()?;
    let root = nav.columns.first()?;
    root.items.iter().position(|it| pred(it.title()))
}

fn miller_click_actions(
    state: &mut AppState,
    column_index: usize,
    item_index: usize,
    activate: bool,
) -> Vec<Action> {
    use crate::app::action::{MillerAction, RadioAction};
    use crate::app::state::BrowseItem;

    // Any click inside a Miller column implies focus has moved out of
    // the leftmost category column — clear that flag so the column
    // focus indicator paints the right column.
    let category_was_focused = state.category_column_focused;
    state.category_column_focused = false;

    // Move selection + focus, then clone the item (and any siblings
    // we'll need) so the rest of the function can re-borrow `state`
    // without conflicting. Plain clicks select only — they don't
    // drill. Drill happens on `activate` (double-click) or via the
    // shared `Enter` / `Right` keyboard handlers. The `prev_focus`
    // guard below preserves the same behaviour when the click is
    // also moving focus to a new column.
    // Snapshot whether anything is open rightward of the clicked
    // column BEFORE we mutate nav. `track_details` is set by an
    // explicit Enter/double-click on a Track and counts as a
    // rightward open thing for the auto-drill rule.
    let pane_open = state.track_details.is_some();
    let (item, track_obj, grouped_album_col, auto_drill) = {
        let Some(nav) = state.browse_nav_mut() else { return Vec::new() };
        let had_child = column_index + 1 < nav.columns.len() || pane_open;
        let Some(col) = nav.columns.get_mut(column_index) else { return Vec::new() };
        col.selected_index = item_index;
        nav.focused_column = column_index;
        // Plain click anywhere is selection-only by default. Stale
        // child cols are truncated. EXCEPTION: when something
        // rightward is already open, clicking a sibling re-fills
        // the rightward child from the new selection — the user
        // has already committed to the drill.
        let auto_drill = !activate && had_child;
        if !activate {
            nav.columns.truncate(column_index + 1);
        }
        let _ = category_was_focused;
        if !activate && !auto_drill {
            return Vec::new();
        }
        let item = nav.columns.get(column_index)
            .and_then(|c| c.items.get(item_index)).cloned();
        // For a Track click in any nav, we need the matching full
        // Track object from the parallel `tracks` array.
        let track_obj = nav.columns.get(column_index)
            .and_then(|c| c.tracks.get(item_index).cloned());
        // For a Playlist+grouped Album click, build the new tracks
        // column locally so we don't have to hit the API.
        let grouped_album_col = if state.browse_category == crate::app::state::BrowseCategory::Playlists {
            let c = state.playlist_nav.columns.get(column_index);
            c.and_then(|c| if c.grouped_by_album { helpers_for_grouped_album(c, item_index) } else { None })
        } else {
            None
        };
        (item, track_obj, grouped_album_col, auto_drill)
    };
    let Some(item) = item else { return Vec::new() };

    // Auto-drill on a non-Track item swaps the rightward column to
    // that item's child — but the track-details pane has no logical
    // re-target (the new column doesn't pick out a single track),
    // so close it. The Track match arm below will re-set
    // `track_details` via OpenTrackDetails.
    if auto_drill && !matches!(item, BrowseItem::Track { .. }) {
        state.track_details = None;
        state.track_pane_focused = false;
        state.track_pane_index = 0;
    }
    let _ = auto_drill;

    let mut arm_drill = true;
    let actions: Vec<Action> = match &item {
        BrowseItem::Artist { key, .. } => vec![Action::Miller(
            MillerAction::LoadArtistAlbumsForMiller { artist_key: key.clone() },
        )],
        BrowseItem::Album { key, title, .. } => {
            // In the Playlists category with `grouped_by_album` set,
            // an Album row is a synthetic group of the playlist's own
            // tracks — drilling fetches them locally rather than
            // hitting the API. Mirrors the TUI mouse handler.
            if let Some(new_col) = grouped_album_col {
                state.playlist_nav.push_column(new_col);
                arm_drill = false;
                Vec::new()
            } else {
                state.library.selected_album_title = title.clone();
                // Per-category dispatch: each nav has its own loader
                // because the loader pushes the new tracks column
                // onto the matching `*_nav.columns`. Genres in
                // particular needs `LoadGenreTracksForMiller` —
                // dispatching `LoadAlbumTracksForMiller` (which
                // pushes to `artist_nav`) leaves the genre column
                // unchanged and silently breaks drill-down.
                let action = match state.browse_category {
                    crate::app::state::BrowseCategory::Genres => {
                        MillerAction::LoadGenreTracksForMiller { album_key: key.clone() }
                    }
                    _ => MillerAction::LoadAlbumTracksForMiller { album_key: key.clone() },
                };
                vec![Action::Miller(action)]
            }
        }
        BrowseItem::Track { .. } => {
            // Tracks are terminal: instead of drilling further or
            // playing, opening a track-details pane to the right of
            // the Miller columns. The pane is replaced (not stacked)
            // by each new track click.
            arm_drill = false;
            match track_obj {
                Some(t) => vec![Action::Browse(crate::app::action::BrowseAction::OpenTrackDetails(t))],
                None => Vec::new(),
            }
        }
        BrowseItem::Playlist { key, .. } => vec![Action::Miller(
            MillerAction::LoadPlaylistTracksForMiller { playlist_key: key.clone() },
        )],
        BrowseItem::Genre { key, .. } => vec![Action::Miller(
            MillerAction::LoadGenreAlbumsForMiller { genre_key: key.clone() },
        )],
        BrowseItem::AllTracks { artist_key, artist_name, .. } => {
            if artist_key == "__all_library__" {
                vec![Action::Miller(MillerAction::LoadAllLibraryTracksForMiller)]
            } else if artist_key == "__all_comp__" {
                vec![Action::Miller(MillerAction::LoadAllCompilationTracksForMiller)]
            } else if let Some(real_key) = artist_key.strip_prefix("__comp_tracks:") {
                vec![Action::Miller(MillerAction::LoadCompilationAllTracksForMiller {
                    artist_key: real_key.to_string(),
                    artist_name: artist_name.clone(),
                })]
            } else {
                vec![Action::Miller(MillerAction::LoadArtistAllTracksForMiller { artist_key: artist_key.clone() })]
            }
        }
        BrowseItem::AllArtists => vec![Action::Miller(MillerAction::LoadAllAlbumsForMiller)],
        BrowseItem::Compilations => vec![Action::Miller(MillerAction::LoadCompilationsForMiller)],
        BrowseItem::CompilationTracks { artist_key, artist_name } => vec![Action::Miller(
            MillerAction::LoadCompilationAlbumsForMiller {
                artist_key: artist_key.clone(),
                artist_name: artist_name.clone(),
            },
        )],
        BrowseItem::ArtistRadio { artist_key, artist_name, .. } => {
            arm_drill = false;
            vec![Action::Radio(RadioAction::StartPlexRadio {
                key: artist_key.clone(),
                title: artist_name.clone(),
            })]
        }
        BrowseItem::GenreCategory { key, .. } => {
            // Drill from column 0's category row (All / Library /
            // Artist / Album / Mood / Style) into column 1 filled
            // with the matching genre list. Mirrors the TUI mouse
            // handler — the GUI used to misroute this to
            // `LoadCategoryTracks` which doesn't change the genre
            // column at all, so the user always saw the merged
            // "all genres" list regardless of which tab they clicked.
            vec![Action::Browse(crate::app::action::BrowseAction::DrillGenreCategory {
                category_key: key.clone(),
            })]
        }
    };

    if arm_drill && !actions.is_empty() {
        state.auto_drill_pending = true;
    }
    actions
}

