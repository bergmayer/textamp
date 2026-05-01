//! Iced messages for the GUI application.
//!
//! Each front-end event (menu click, key press, window resize, async
//! task result) collapses to a single `GuiMessage` value that `update()`
//! processes and then dispatches through the shared `Action` router.

use crossterm::event::KeyEvent;

use crate::app::state::{SettingsSection, VisualizerTab};
use crate::app::theme::ThemeName;
use crate::app::{Action, Event};
use crate::ui_gui::widgets::menu_bar::TopMenu;

/// Which `state.popups.*` field a `CloseStatePopup` should clear.
#[derive(Debug, Clone, Copy)]
pub enum StatePopupKind {
    Sort,
    Search,
    RadioLauncher,
    AdventureLauncher,
    ArtistRadioPicker,
}

#[derive(Debug, Clone)]
pub enum GuiMessage {
    /// No-op; used as filler by stub code paths.
    Noop,

    /// The Iced window resized to the given dimensions (physical px).
    WindowResized { width: u32, height: u32 },

    /// Open the given top-level menu (and close any previous one).
    MenuOpen(TopMenu),

    /// Close any open top-level menu.
    MenuClose,

    /// User clicked a menu item — close the menu and dispatch the action.
    MenuItemClick(Action),

    /// User clicked a menu item whose behaviour depends on state context
    /// (e.g. Similar, Related, Random Album). Synthesises the equivalent
    /// keystroke and feeds it through the shared key-input handler.
    MenuKeyClick(KeyEvent),

    /// User clicked a primary-view tab — dispatch the accompanying Actions.
    TabClick(Vec<Action>),

    /// Show the About popup (GUI-only, no shared state).
    ShowAbout,
    /// Dismiss the About popup.
    HideAbout,

    /// User edited the quick-filter text input on the tab row. The payload
    /// is the new full value of the input field; the app rebuilds the list
    /// filter state from scratch (active + query + column).
    FilterChanged(String),

    /// Toggle the "show cover art" flag globally (mirrors the TUI's Sort
    /// popup → Artwork toggle). Propagates the new value to every Miller
    /// column's `artwork_visible`.
    ToggleCoverArt,

    /// A Miller column scrollable emitted a scroll update. Used by
    /// `App::snap_focused_column_into_view` to decide whether a selection
    /// change needs to adjust the scroll offset, so arrow-key navigation
    /// only scrolls the viewport when the selected row would move off
    /// screen (same feel as the TUI).
    MillerScroll {
        column_index: usize,
        offset_y: f32,
        bounds_h: f32,
        content_h: f32,
    },

    /// A key press translated into a `crossterm::KeyEvent`. Resolved to
    /// `Action`s in `update()` where we have `&mut AppState` + `&Config`
    /// to drive the shared `key_input::handle_key` dispatcher.
    KeyPress(KeyEvent),

    /// Live keyboard modifier state, fired by an
    /// `iced::keyboard::Event::ModifiersChanged` subscription. Mouse
    /// click events in iced 0.13 don't carry modifier info; the app
    /// caches the latest value here so a subsequent click handler can
    /// distinguish plain / shift / cmd clicks for multi-select.
    ModifiersChanged(iced::keyboard::Modifiers),

    /// "Play Random Album" sidebar / Tools-menu button — picks one
    /// album at random from the active library and queues it as a
    /// regular one-shot (clear queue, load tracks, play). Distinct
    /// from "Random Album Radio" in the stations popup, which is the
    /// *continuous* station that keeps queuing fresh random albums.
    /// Implemented in `update()` rather than as a shared `Action` so
    /// the random selection stays out of the cross-platform core.
    PlayOneRandomAlbum,

    /// Result of an async `client.get_similar_tracks` fetch fired by
    /// the Browse track-details pane. The pane shows sonically-
    /// similar tracks under the metadata; the Tick handler triggers
    /// the fetch on first sighting of a new track and stores the
    /// result in `App::track_pane_similar`.
    TrackPaneSimilarLoaded {
        track_key: String,
        tracks: Vec<crate::plex::models::Track>,
    },

    /// Right-click on a row that carries a full `Track` payload but
    /// isn't anchored to a Miller column index — currently the
    /// "Sonically Similar" rows in the Browse track-details pane.
    /// Opens the standard track context menu (Play / Play next /
    /// Add to queue / Show Similar / Related / Open in Library / …)
    /// at the cursor position.
    OpenStandaloneTrackContextMenu(Box<crate::plex::models::Track>),
    /// Open the command palette overlay. Fired from the Tools →
    /// Command Palette menu item (Cmd+K) — a guaranteed-to-work
    /// entry point that goes through muda's accelerator handler
    /// rather than relying on the iced keyboard subscription
    /// (which can miss keystrokes if a focused widget swallows
    /// them first).
    OpenCommandPalette,
    /// Same menu but for a "floating" track — i.e. one that's not
    /// anchored in the user's current Miller-drill context (e.g. a
    /// similar-track row in the track-details pane). Forces "Open in
    /// Library" near the top regardless of the active view.
    OpenFloatingTrackContextMenu(Box<crate::plex::models::Track>),

    /// Click on a playlist row that lives in the leftmost category
    /// column (under the Library / Genres / Folders header rows).
    /// Switches the browse category to Playlists and drills into the
    /// clicked playlist's tracks in one motion, so the user doesn't
    /// have to step through "click Playlists, then click the row".
    OpenPlaylistFromCategory {
        playlist_key: String,
        title: String,
    },

    /// A single `Action` (from a menu item click, widget callback, etc.).
    Action(Action),

    /// Click on a Miller column row. `activate = true` means the row was
    /// already selected, i.e. a second click / double-click / Enter —
    /// mirrors the TUI's click-already-selected = activate semantics.
    MillerSelect {
        column_index: usize,
        item_index: usize,
        activate: bool,
    },

    /// An `Event` posted by a background task on the shared mpsc channel.
    /// Handled identically to the TUI event path: translate to Actions and
    /// dispatch.
    CoreEvent(Event),

    /// Periodic tick from the Iced subscription. Drives playback position
    /// advancement (the TUI does this in its event-loop body), the
    /// `Event::Tick` follow-up actions (marquee / toast expiry / visualizer
    /// safety-net loading / playback progress report) and track-end
    /// detection via `AudioBackend::is_finished`.
    Tick,

    /// User clicked one of the Now Playing visualizer tabs. Switches
    /// `state.visualizer_tab` and kicks off the matching data load.
    SetVisualizerTab(VisualizerTab),

    /// Track cursor position so a subsequent right-click can place its
    /// context menu at the cursor point. Fires off an app-wide mouse_area
    /// wrapping the main view.
    MouseMoved { x: f32, y: f32 },

    /// User right-clicked a Miller-column row. The handler builds the
    /// appropriate context-menu entries based on the item kind and opens
    /// the menu at the last-known cursor position.
    OpenMillerContextMenu { column_index: usize, item_index: usize },

    /// User clicked an entry in the active context menu — run its actions
    /// and close the menu.
    ContextMenuClick(Vec<Action>),

    /// User clicked outside the context menu (or pressed Escape).
    CloseContextMenu,

    /// User clicked a row in the Sort popup. Sets the popup's
    /// `selected_index` and applies the option in one motion — mirrors
    /// the keyboard Enter path.
    SortPopupClick(usize),

    /// User clicked a track row in the Radio Launcher popup.
    RadioLauncherClick(usize),

    /// User clicked a row in the Adventure Launcher popup.
    AdventureLauncherClick(usize),

    /// User clicked a row in the Artist Radio Picker popup.
    ArtistRadioPickerClick(usize),

    /// User clicked a search result row in the search popup.
    SearchPopupClick(usize),

    /// User bumped the UI scale up or down from the Settings view.
    /// Delta is added to the current value and clamped to `UI_SCALE_MIN..=UI_SCALE_MAX`.
    AdjustUiScale(f32),

    /// User edited the username text input on the auth screen.
    AuthUsernameChanged(String),

    /// User edited the password text input on the auth screen.
    AuthPasswordChanged(String),

    /// User clicked the "Radio…" button in the queue view. Opens a modal
    /// popup with the full station navigation.
    OpenStationsPopup,

    /// User dismissed the stations popup (X button, Esc, or launched a
    /// station which closes on play).
    CloseStationsPopup,

    /// User clicked a playable station in the stations popup. Dispatches
    /// the given actions and closes the popup in one motion.
    PlayStationAndClose(Vec<Action>),

    /// User clicked an album/playlist thumbnail in a Miller row. Opens a
    /// full-size art popup and kicks off a high-resolution fetch in the
    /// background.
    OpenArtPopup { key: String, thumb_path: String },

    /// Dismiss the full-size art popup.
    CloseArtPopup,

    /// Background high-resolution artwork fetch completed. Replaces the
    /// cached bytes with the new higher-quality version so the popup
    /// image re-renders sharp.
    HiresArtLoaded { key: String, data: Vec<u8> },

    /// Context-menu "Show Similar" entry. Dispatches the supplied Load
    /// actions (which populate `state.similar`) and raises the popup
    /// overlay instead of navigating to the full-screen Similar view.
    ShowSimilarPopup(Vec<Action>),

    /// Dismiss the similar popup.
    CloseSimilarPopup,

    /// User clicked Yes on the active confirm dialog. Handler reads
    /// `state.popups.confirm_dialog.on_confirm`, dispatches the
    /// matching action, and clears the dialog.
    ConfirmDialogYes,

    /// User clicked No / Cancel on the active confirm dialog.
    ConfirmDialogNo,

    /// User typed into the input dialog — updates the dialog's
    /// `input` field in state.
    InputDialogChanged(String),

    /// User clicked OK on the input dialog. Handler reads
    /// `state.popups.input_dialog.action_type` and dispatches the
    /// matching action with the current input.
    InputDialogSubmit,

    /// User clicked Cancel / Esc on the input dialog.
    InputDialogCancel,

    /// Dismiss the artist-bio popup from a click.
    CloseBioPopup,

    /// Dismiss a state-tracked popup (sort, search, radio launcher,
    /// adventure launcher, artist-radio picker) via its Close button.
    /// The variant tells the handler which one to clear.
    CloseStatePopup(StatePopupKind),

    /// Move the queue row at `idx` one slot up. Sets
    /// `state.list_state.queue_index = idx` first so the shared
    /// `QueueAction::MoveQueueTrackUp` handler (which operates on
    /// that single cursor) targets the right row.
    MoveQueueRowUp(usize),

    /// Move the queue row at `idx` one slot down.
    MoveQueueRowDown(usize),

    /// Remove the queue row at `idx` from the queue.
    RemoveQueueRow(usize),

    /// Context menu "Related Artists". Loads state.related and opens
    /// a popup overlay. Same pattern as `ShowSimilarPopup`: the
    /// shared dispatcher flips `state.view = View::Related`, we
    /// capture the previous view and snap back.
    ShowRelatedPopup(Vec<Action>),

    /// Dismiss the Related popup.
    CloseRelatedPopup,

    /// Open the Settings popup overlay.
    OpenSettingsPopup,

    /// Dismiss the Settings popup.
    CloseSettingsPopup,

    /// Clicking an artist row in the Similar / Related popup — close
    /// the popup, switch to Library, drill into that artist. This is
    /// the "take me to this artist" affordance.
    NavigateToArtist { artist_key: String },

    /// Settings popup tab picker — swap the active section.
    SetSettingsSection(SettingsSection),

    /// User picked a theme in Settings → View Options.
    SetTheme(ThemeName),

    /// Tools → Retry Audio Device. Re-tries `RodioBackend::new()` and
    /// swaps it into the audio player if the device is now available,
    /// dismissing the "Audio unavailable" banner.
    RetryAudio,

    /// User right-clicked a track row in the Folders Miller column.
    /// `row_index` indexes into the focused `FolderColumn::items`.
    /// The handler builds a context menu with Play / Show Artist Bio
    /// / Open in Library — the Folders nav uses a different data
    /// model than the rest of Miller browsing so the regular
    /// `OpenMillerContextMenu` path isn't reachable here.
    OpenFolderContextMenu { row_index: usize },

    /// User left-clicked a folder/track row in the Folders Miller
    /// column. Sets the folder nav's focused column + selection
    /// BEFORE dispatching the drill (NavigateIntoFolder) or play
    /// (PlayFolderTrack) — without that step `push_column`'s
    /// `truncate_right` uses whichever column was previously focused
    /// and leaves drill columns from a different branch alive.
    FolderRowClick { column_index: usize, row_index: usize, is_folder: bool },

    /// User clicked the empty area at the top of a Miller column
    /// (the title bar / chrome — not a row). Move keyboard focus
    /// into that column without changing its row selection or
    /// truncating drill columns to its right. Lets the user pick
    /// which column the arrow keys / sort menu act on without
    /// clicking a row first.
    FocusMillerColumn { column_index: usize },

    /// User clicked the small "x" in a Miller column / track-details
    /// pane header. Focuses the targeted column first (so the close
    /// helper drops the right one) and runs the same logic as Cmd+W.
    /// `column_index = None` is reserved for the track-details pane,
    /// where there is no Miller column to focus — only
    /// `track_pane_open` gets flipped off.
    CloseMillerColumn { column_index: Option<usize> },

    /// User clicked anywhere inside the track-details pane (chrome,
    /// padding, or even the artwork). Treats the pane as a column —
    /// claims focus from the cat col / miller cols so the
    /// single-focused-column rule renders the right thing. The
    /// per-element messages (Play Track button, similar-row click,
    /// close X) keep their own message types and are NOT routed
    /// through this — they already do the right thing.
    FocusTrackPane,

    /// Click on a Sonically Similar row in the track pane. First
    /// click highlights only — sets `track_pane_index` to the row;
    /// a second click on the same row (or pressing Enter on it)
    /// fires `OpenInLibrary` for that track's album, mirroring the
    /// click-on-highlighted = Enter rule used everywhere else.
    /// `pane_index` is 1-based — index 0 is reserved for the Play
    /// button.
    SimilarRowClick { pane_index: usize },

    /// User pressed the mouse on queue row `idx`. Records the index as
    /// the drag source AND moves `list_state.queue_index` to it so the
    /// Delete shortcut targets the clicked row. Hovers over other rows
    /// update the drop target via `QueueDragOver`. Whether the gesture
    /// is a click (play) or a drag (reorder) is decided on release.
    QueueDragStart(usize),

    /// Cursor moved over queue row `idx` while a drag is in progress.
    QueueDragOver(usize),

    /// Mouse released anywhere — commits the gesture: same row → play
    /// that track; different row → reorder source to target. Cleared
    /// from a window-level `iced::Event::Mouse` subscription so releases
    /// outside any row also end the drag.
    QueueDragEnd,

    /// Right-click on a queue row → open a context menu with Play /
    /// Play next / Add to end / Remove options.
    OpenQueueContextMenu { row_index: usize },

    /// Click on a letter in the alphabet strip (between the category
    /// column and the first Miller column). Jumps the focused root
    /// browse list (Artists/Playlists/Genres) to the first item whose
    /// sort key starts with this character. `'0'` matches digits and
    /// `'%'` matches symbols (any non-alphanumeric).
    AlphabetJump(char),

    /// Open the DJ Modes picker popup (sidebar button on Now Playing).
    OpenDjModesPopup,
    /// Dismiss the DJ Modes popup (Close button or outside click).
    CloseDjModesPopup,

    /// Open the Remix Tools popup (sidebar button on Now Playing).
    OpenRemixToolsPopup,
    /// Dismiss the Remix Tools popup.
    CloseRemixToolsPopup,

    /// User clicked a one-shot action in the Remix Tools popup
    /// (Remix: Gemini, Clear queue, …). Runs the action and closes
    /// the popup in one motion. Mirrors `PlayStationAndClose`.
    RemixToolClick(Action),

    /// Open the User Guide popup (Help → User Guide). Renders
    /// `README-GUI.md` as a scrollable modal.
    OpenUserGuide,
    /// Dismiss the User Guide popup.
    CloseUserGuide,

    /// Open the Keyboard Shortcuts popup (Help → Keyboard Shortcuts).
    OpenKeyboardShortcuts,
    /// Dismiss the Keyboard Shortcuts popup.
    CloseKeyboardShortcuts,
}
