//! Browse view key handling.
//!
//! Handles all browse-related keyboard input including:
//! - Main browse view dispatch
//! - Folder browsing (Miller columns)
//! - Station browsing (Miller columns)
//! - Artist browsing (dynamic Miller columns)
//! - Genre browsing (dynamic Miller columns)
//! - Playlist browsing (dynamic Miller columns)
//! - Common navigation keys for BrowseNavigationState

use crate::app::action::*;
use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::Action;
use crate::app::state::{
    BrowseCategory, Focus, RightPanelMode, View,
};
use crate::app::AppState;
use super::super::helpers;

/// Handle Browse view keys (CUA-style).
pub(super) fn handle_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Inline list filter mode - handle filter-specific keys
    if state.list_filter.active {
        // Check if current focus is on the filter's target column
        let focused_on_filter_column = match state.list_filter.category {
            BrowseCategory::Library => state.artist_nav.focused_column == state.list_filter.column,
            BrowseCategory::Playlists => state.playlist_nav.focused_column == state.list_filter.column,
            BrowseCategory::Folders => {
                state.folder_state.as_ref()
                    .map(|fs| fs.focused_column == state.list_filter.column)
                    .unwrap_or(false)
            }
            cat if cat.is_tag_section() => state.tag_nav.focused_column == state.list_filter.column,
            _ => false,
        };

        match key.code {
            // Esc always deactivates filter
            KeyCode::Esc => {
                return vec![SearchAction::DeactivateListFilter.into()];
            }
            // Backspace deletes from filter query
            KeyCode::Backspace => {
                return vec![SearchAction::DeleteListFilterChar.into()];
            }
            // Up/Down navigate the filter results unconditionally.
            // Even when focus has wandered off to a child column,
            // pressing Up returns to the filter column AND moves
            // selection in one step — instead of the previous
            // two-step where the first Up only re-focused and you
            // had to press Up again to actually navigate.
            KeyCode::Up => {
                if !focused_on_filter_column {
                    truncate_filter_right_columns(state);
                }
                return vec![SearchAction::FilteredListUp.into()];
            }
            KeyCode::Down => {
                if !focused_on_filter_column {
                    truncate_filter_right_columns(state);
                }
                return vec![SearchAction::FilteredListDown.into()];
            }
            // Enter and Right on the filter column drill into the
            // highlighted filtered row in one step:
            // `SelectFilteredItem` syncs the column's selected_index
            // to the filter's match (the column may have drifted),
            // deactivates the filter, then dispatches the same drill
            // action a normal Enter would. Without this, Enter falls
            // through to the column's own handler and drills using a
            // stale `selected_index` (or, with filter results still
            // racing in, no drill at all), producing the user-visible
            // "first Enter undoes the filter, second Enter drills"
            // glitch.
            KeyCode::Enter | KeyCode::Right if focused_on_filter_column => {
                return vec![SearchAction::SelectFilteredItem.into()];
            }
            // Typing appends to filter query (only unmodified chars)
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) => {
                return vec![SearchAction::AppendListFilterChar(c).into()];
            }
            // Left/Esc on non-filter column: deactivate and fall through
            KeyCode::Left if !focused_on_filter_column => {
                state.list_filter.deactivate();
            }
            // Other keys fall through to normal handling
            _ => {}
        }
    }

    // Activate filter with / key (when not in filter mode and not on category column)
    if key.code == KeyCode::Char('/') && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !state.category_column_focused {
        return vec![SearchAction::ActivateListFilter.into()];
    }

    // Tab toggles between the Library (Browse) and the combined
    // Queue / Now Playing screen. From Browse this means jumping
    // straight to Now Playing — the intermediate "Playlists" /
    // "Queue" stops are reachable from the category column and the
    // command palette respectively, so the dedicated Tab cycle
    // through every view was redundant.
    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            state.set_view(View::NowPlaying);
            return vec![];
        }
        _ => {}
    }

    // Alphabet strip has keyboard focus — Up/Down moves the
    // selected letter, Enter applies the jump, Right hands focus
    // off to the artist column, Left/Esc returns to the category
    // column. Falls through if the strip isn't actually on screen
    // (category change can leave the focus flag stranded).
    if state.alphabet_strip_focused {
        if state.alphabet_strip_visible() {
            return handle_alphabet_strip_keys(key, state);
        } else {
            state.alphabet_strip_focused = false;
        }
    }

    // Track-details pane has keyboard focus — Up/Down moves
    // between the Play button and the Sonically-Similar rows;
    // Enter activates; Left/Esc returns to the focused Miller
    // column. Reset the flag when no track is focused (e.g. user
    // moved selection elsewhere) so the pane handler doesn't
    // strand on stale state.
    if state.track_pane_focused {
        if state.focused_track().is_some() {
            return handle_track_pane_keys(key, state);
        } else {
            state.track_pane_focused = false;
            state.track_pane_index = 0;
        }
    }

    // Category column navigation (column 0 in browse view)
    if state.category_column_focused {
        return handle_category_column_keys(key, state);
    }

    // Handle Folders category separately (Miller columns view)
    if state.browse_category == BrowseCategory::Folders {
        return handle_folder_browse_keys(key, state);
    }

    // Handle Artists category with Miller columns when artist_nav is populated
    if state.browse_category == BrowseCategory::Library && !state.artist_nav.is_empty() {
        return handle_artist_browse_keys(key, state);
    }

    // Handle Playlists category with Miller columns when playlist_nav is populated
    if state.browse_category == BrowseCategory::Playlists {
        if !state.playlist_nav.is_empty() {
            return handle_playlist_browse_keys(key, state);
        }
    }

    // Handle Genres category with Miller columns (Genre | Albums | Tracks)
    if state.browse_category.is_tag_section() {
        return handle_genre_browse_keys(key, state);
    }

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Settings
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        // Navigation (Tab is handled above, before category-specific handlers)
        KeyCode::Up => vec![DataAction::ListUp.into()],
        KeyCode::Down => vec![DataAction::ListDown.into()],
        KeyCode::PageUp => vec![DataAction::ListPageUp.into()],
        KeyCode::PageDown => vec![DataAction::ListPageDown.into()],
        KeyCode::Home => vec![DataAction::ListTop.into()],
        KeyCode::End => vec![DataAction::ListBottom.into()],

        // Selection/Action - depends on focus and current mode
        KeyCode::Enter | KeyCode::Right => {
            if state.focus == Focus::Left {
                // Left panel: depends on category
                match state.browse_category {
                    BrowseCategory::Library => {
                        vec![DataAction::LoadArtistAlbums.into()]
                    }
                    BrowseCategory::Playlists => {
                        vec![DataAction::LoadCategoryTracks.into()]
                    }
                    BrowseCategory::Folders => {
                        vec![FolderAction::LoadFolderRoot.into()]
                    }
                    cat if cat.is_tag_section() => {
                        vec![BrowseAction::LoadTagAlbums { replace_child: false }.into()]
                    }
                    _ => vec![],
                }
            } else {
                // Right panel: depends on mode
                match state.library.right_panel_mode {
                    RightPanelMode::ArtistAlbums => {
                        // Index 0 = "All Tracks", otherwise album
                        if state.list_state.right_albums_index == 0 {
                            vec![DataAction::LoadArtistAllTracks.into()]
                        } else {
                            vec![DataAction::LoadSelectedAlbumTracks.into()]
                        }
                    }
                    RightPanelMode::AlbumTracks | RightPanelMode::CategoryTracks => {
                        // Track selected -> play it
                        vec![QueueAction::PlayTrackFromCategory(state.list_state.tracks_index).into()]
                    }
                    RightPanelMode::CategoryAlbums => {
                        // Album selected in genre view -> load album tracks
                        if let Some(album) = state.library.tag_albums.get(state.library.tag_albums_index).cloned() {
                            state.library.selected_album_title = album.title.clone();
                            state.search.pending_album_key = Some(album.rating_key.clone());
                            vec![DataAction::LoadAlbumTracks { rating_key: album.rating_key }.into()]
                        } else {
                            vec![]
                        }
                    }
                    RightPanelMode::Empty => vec![],
                }
            }
        }
        KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
            if state.focus == Focus::Right {
                // Check if we should go back to album list (from tracks view)
                if state.library.right_panel_mode == RightPanelMode::AlbumTracks {
                    // If we came from a genre album, go back to CategoryAlbums
                    if state.browse_category.is_tag_section() {
                        state.library.right_panel_mode = RightPanelMode::CategoryAlbums;
                        state.library.selected_album_tracks.clear();
                        vec![]
                    } else {
                        vec![DataAction::GoBackInRightPanel.into()]
                    }
                } else {
                    vec![NavigationAction::ToggleFocus.into()]
                }
            } else {
                vec![]
            }
        }

        // Alphabet jumping - jump to first item starting with letter
        // Allow with no modifiers or just SHIFT (for uppercase)
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            super::jump_to_letter(state, c);
            vec![]
        }

        _ => vec![],
    }
}

/// Handle keys when the category selector column (column 0) is focused.
///
/// The column has the GUI's compound layout: Library / Genres /
/// Folders pinned at the top, then each playlist as its own row.
/// Navigation operates on the flat `state.category_rows()` index
/// space; Enter dispatches the right action depending on which
/// CategoryRow variant is selected.
fn handle_category_column_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::state::CategoryRow;

    let rows = state.category_rows();
    let num_rows = rows.len().max(1);

    match key.code {
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        KeyCode::Up => {
            // Walk back over Divider rows so navigation feels like
            // there's nothing between the chunks. Auto-drill the
            // rightward content to mirror the new section, but keep
            // focus on the sections column so the user can keep
            // arrow-keying.
            let mut idx = state.category_column_index;
            while idx > 0 {
                idx -= 1;
                if !matches!(rows.get(idx), Some(CategoryRow::Divider)) {
                    state.category_column_index = idx;
                    return drill_section_row(state, false);
                }
            }
            vec![]
        }
        KeyCode::Down => {
            let mut idx = state.category_column_index;
            while idx + 1 < num_rows {
                idx += 1;
                if !matches!(rows.get(idx), Some(CategoryRow::Divider)) {
                    state.category_column_index = idx;
                    return drill_section_row(state, false);
                }
            }
            vec![]
        }
        KeyCode::Home => {
            // First non-divider row.
            let first = rows.iter().position(|r| !matches!(r, CategoryRow::Divider)).unwrap_or(0);
            state.category_column_index = first;
            drill_section_row(state, false)
        }
        KeyCode::End => {
            let last = rows.iter().rposition(|r| !matches!(r, CategoryRow::Divider)).unwrap_or(num_rows - 1);
            state.category_column_index = last;
            drill_section_row(state, false)
        }

        // Right moves focus to the alphabet strip when it's
        // visible, so Right/Right walks cat → strip → artists.
        // When the strip isn't on screen, Right falls through to the
        // drill behaviour shared with Enter.
        KeyCode::Right if state.alphabet_strip_visible() => {
            state.category_column_focused = false;
            state.alphabet_strip_focused = true;
            return vec![];
        }

        // Right/Enter drills into the selected row and takes focus.
        KeyCode::Right | KeyCode::Enter => drill_section_row(state, true),

        // Letter jump still works for the top three categories.
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = c.to_ascii_lowercase();
            if let Some(idx) = rows.iter().position(|row| match row {
                CategoryRow::Category(cat) => cat.name().starts_with(lower),
                CategoryRow::Playlist(i) => state.library.playlists.get(*i)
                    .map(|p| p.title.to_lowercase().starts_with(lower))
                    .unwrap_or(false),
                CategoryRow::Divider => false,
            }) {
                state.category_column_index = idx;
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Drill action for the currently-highlighted sections-column row.
///
/// `take_focus = true` is the explicit Right / Enter drill: focus
/// shifts onto the rightward content. `take_focus = false` is the
/// auto-drill that fires on Up / Down / Home / End sweeps: the
/// rightward content updates to mirror the highlighted section, but
/// keyboard focus stays on the sections column. The lazy-art gate is
/// raised on auto-drill so a held arrow key doesn't fire one art
/// fetch per keypress.
fn drill_section_row(state: &mut AppState, take_focus: bool) -> Vec<Action> {
    use crate::app::state::CategoryRow;
    let rows = state.category_rows();
    let Some(row) = rows.get(state.category_column_index).copied() else {
        return vec![];
    };
    let auto_drill = !take_focus;
    if auto_drill {
        note_motion_for_lazy_art(state);
    } else {
        state.category_column_focused = false;
    }
    match row {
        CategoryRow::Category(cat) => {
            vec![NavigationAction::SetCategory {
                category: cat,
                preserve_sections_focus: auto_drill,
            }.into()]
        }
        CategoryRow::Playlist(i) => {
            let Some(p) = state.library.playlists.get(i) else { return vec![] };
            let key = p.rating_key.clone();
            let title = p.title.clone();
            // `set_browse_category` rewrites `category_column_index`
            // to the position of `Playlists` in `BrowseCategory::all()`.
            // That position has nothing to do with the playlist row
            // we just navigated to in `category_rows()` — saving and
            // restoring the index keeps the highlight where the user
            // moved it, otherwise Down past the categories block
            // would teleport the cursor on every keypress.
            let row_idx = state.category_column_index;
            state.set_browse_category(BrowseCategory::Playlists, auto_drill);
            state.category_column_index = row_idx;
            if let Some(col) = state.playlist_nav.columns.get_mut(0) {
                if let Some(idx) = col.items.iter().position(|it| it.key() == key.as_str()) {
                    col.selected_index = idx;
                }
            }
            if take_focus {
                state.playlist_nav.focused_column = 0;
            }
            state.playlist_nav.truncate_right();
            state.library.selected_album_title = title;
            vec![MillerAction::LoadPlaylistTracksForMiller {
                playlist_key: key,
                replace_child: auto_drill,
            }.into()]
        }
        CategoryRow::Divider => vec![],
    }
}

/// Handle keys when the alphabet jump strip has keyboard focus.
///
/// Up/Down moves the highlighted letter and auto-scrolls the artist
/// list to match (so the user can "scrub" through the alphabet);
/// Enter is a redundant explicit re-jump; Right hands focus off to
/// the artist root column without changing scroll; Left/Esc returns
/// focus to the category column.
///
/// When the artist column is sorted descending, Z is at the visual
/// top of the strip and % at the bottom — Up/Down arrows still move
/// in the visual direction, so they flip relative to the natural
/// alphabet index.
fn handle_alphabet_strip_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::app::handlers::helpers::{alphabet_jump, ALPHABET_STRIP_LETTERS};
    let n = ALPHABET_STRIP_LETTERS.len();
    let descending = state
        .artist_nav
        .columns
        .first()
        .map_or(false, |c| !c.sort_ascending);

    let mut moved = false;
    match key.code {
        KeyCode::F(1) | KeyCode::Char('?') => return vec![NavigationAction::SetView(View::Help).into()],
        KeyCode::F(2) => return vec![SettingsAction::OpenSettings.into()],

        KeyCode::Up => {
            if descending {
                state.alphabet_strip_index = (state.alphabet_strip_index + 1).min(n - 1);
            } else {
                state.alphabet_strip_index = state.alphabet_strip_index.saturating_sub(1);
            }
            moved = true;
        }
        KeyCode::Down => {
            if descending {
                state.alphabet_strip_index = state.alphabet_strip_index.saturating_sub(1);
            } else {
                state.alphabet_strip_index = (state.alphabet_strip_index + 1).min(n - 1);
            }
            moved = true;
        }
        KeyCode::Home => {
            // Visual top of the strip.
            state.alphabet_strip_index = if descending { n - 1 } else { 0 };
            moved = true;
        }
        KeyCode::End => {
            // Visual bottom of the strip.
            state.alphabet_strip_index = if descending { 0 } else { n - 1 };
            moved = true;
        }

        KeyCode::Enter | KeyCode::Right => {
            // Transfer focus to the artist root column AND snap the
            // selection to the first artist matching the highlighted
            // letter. The previous "preserve selection" behaviour
            // sent the user from "I picked D on the strip" straight
            // back to whatever artist had been selected before
            // (often row 0), which felt like the list was scrolling
            // back to the top — what the user actually wanted is the
            // highlight to land on the first D artist so they can
            // immediately drill in or move down.
            if let Some(&ch) = ALPHABET_STRIP_LETTERS.get(state.alphabet_strip_index) {
                if let Some(target) = crate::app::handlers::helpers::alphabet_target_index(state, ch) {
                    if let Some(nav) = state.browse_nav_mut() {
                        if let Some(col) = nav.columns.get_mut(0) {
                            col.selected_index = target;
                        }
                    }
                }
            }
            state.alphabet_strip_focused = false;
            state.artist_nav.focused_column = 0;
        }
        KeyCode::Left | KeyCode::Esc | KeyCode::Backspace => {
            state.alphabet_strip_focused = false;
            state.category_column_focused = true;
        }

        // Letter typed while on the strip = quick jump to that letter
        // (matches type-ahead muscle memory from any other column).
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            let target = c.to_ascii_lowercase();
            if let Some(idx) = ALPHABET_STRIP_LETTERS.iter().position(|&x| x == target) {
                state.alphabet_strip_index = idx;
                moved = true;
            }
        }

        _ => {}
    }

    // Scroll the artist column to track the highlighted letter.
    if moved {
        if let Some(&ch) = ALPHABET_STRIP_LETTERS.get(state.alphabet_strip_index) {
            let _ = alphabet_jump(state, ch);
        }
    }
    vec![]
}

/// Handle keys when the right-side track-details pane has keyboard
/// focus. Up/Down moves between the Play button (index 0) and the
/// Sonically-Similar rows (index 1..=N); Enter triggers the
/// highlighted action (play track / drill into similar track's
/// album); Left, Esc, or Backspace returns focus to the focused
/// Miller column.
fn handle_track_pane_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    let track = match state.focused_track() {
        Some(t) => t.clone(),
        None => {
            state.track_pane_focused = false;
            state.track_pane_index = 0;
            return vec![];
        }
    };
    let similar_count = state
        .track_pane_similar
        .get(&track.rating_key)
        .map(|v| v.len())
        .unwrap_or(0);
    let max_idx = similar_count; // 0 = play, 1..=N = similar

    match key.code {
        KeyCode::F(1) | KeyCode::Char('?') => return vec![NavigationAction::SetView(View::Help).into()],
        KeyCode::F(2) => return vec![SettingsAction::OpenSettings.into()],

        KeyCode::Up => {
            state.track_pane_index = state.track_pane_index.saturating_sub(1);
        }
        KeyCode::Down => {
            state.track_pane_index = (state.track_pane_index + 1).min(max_idx);
        }
        KeyCode::Home => state.track_pane_index = 0,
        KeyCode::End => state.track_pane_index = max_idx,

        KeyCode::Enter => {
            if state.track_pane_index == 0 {
                return vec![QueueAction::PlayTrack(track).into()];
            }
            // Open the command palette as the contextual menu for the
            // highlighted similar-track row. The palette's context-
            // aware section reads `palette_target_track()`, which
            // returns the similar track when `track_pane_focused &&
            // track_pane_index > 0` — so the user gets Play Track /
            // Open in Library (always shown for similar rows) /
            // Artist Bio / Sonic Adventure / external search at the
            // top of the list.
            let _ = track;
            crate::ui::command_palette::open(state);
            return vec![];
        }

        KeyCode::Left | KeyCode::Esc | KeyCode::Backspace => {
            state.track_pane_focused = false;
            state.track_pane_index = 0;
        }

        _ => {}
    }
    vec![]
}

/// Handle folder browsing mode keys (Miller columns style).
pub(super) fn handle_folder_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    use crate::services::FolderItemType;

    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => vec![NavigationAction::SetView(View::Help).into()],

        // Settings
        KeyCode::F(2) => vec![SettingsAction::OpenSettings.into()],

        // Up/Down/PgUp/PgDn/Home/End — selection-only. Keyboard
        // nav never auto-drills; rightward columns get truncated so
        // the path stays coherent and the user re-drills with
        // Enter / Right when they're ready.
        KeyCode::Up => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_up();
                folder_state.truncate_right_columns();
            }
            return vec![];
        }
        KeyCode::Down => {
            if let Some(ref mut folder_state) = state.folder_state {
                folder_state.move_down();
                folder_state.truncate_right_columns();
            }
            return vec![];
        }
        KeyCode::PageUp => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.selected_index.saturating_sub(10);
                }
                folder_state.truncate_right_columns();
            }
            return vec![];
        }
        KeyCode::PageDown => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    let max = col.items.len().saturating_sub(1);
                    col.selected_index = (col.selected_index + 10).min(max);
                }
                folder_state.truncate_right_columns();
            }
            return vec![];
        }
        KeyCode::Home => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = 0;
                }
                folder_state.truncate_right_columns();
            }
            return vec![];
        }
        KeyCode::End => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    col.selected_index = col.items.len().saturating_sub(1);
                }
                folder_state.truncate_right_columns();
            }
            return vec![];
        }

        // Right/Enter - go into selected folder; only Enter plays tracks
        KeyCode::Enter | KeyCode::Right => {
            if let Some(ref mut folder_state) = state.folder_state {
                // Check if there's a non-empty column to the right we can move to.
                // Skip empty placeholder columns — they exist only for visual layout
                // and shouldn't intercept navigation.
                let next_col_has_items = folder_state.columns
                    .get(folder_state.focused_column + 1)
                    .map_or(false, |col| !col.items.is_empty());
                if next_col_has_items && folder_state.focus_right() {
                    return vec![];
                }

                // Otherwise, load the selected item
                if let Some(item) = folder_state.selected_item().cloned() {
                    match item.item_type {
                        FolderItemType::Folder => {
                            return vec![FolderAction::NavigateIntoFolder {
                                folder_key: item.key,
                                replace_child: false,
                            }.into()];
                        }
                        FolderItemType::Track if key.code == KeyCode::Enter => {
                            // Enter: play this track + all following tracks (replaces queue)
                            return vec![FolderAction::PlayFolderTracks.into()];
                        }
                        _ => {}
                    }
                }
            }
            vec![]
        }

        // Left/Backspace - move focus to previous column or category column
        KeyCode::Left | KeyCode::Backspace => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                } else {
                    state.focus_category_column();
                }
            } else {
                state.focus_category_column();
            }
            vec![]
        }

        // Escape - go back, or truncate child columns at root
        KeyCode::Esc => {
            if let Some(ref mut folder_state) = state.folder_state {
                if folder_state.can_go_left() {
                    folder_state.focus_left();
                    return vec![];
                }
                // At root column: truncate child columns, keep placeholder
                if folder_state.columns.len() > 1 {
                    folder_state.columns.truncate(1);
                    folder_state.ensure_placeholder();
                    return vec![];
                }
            }
            vec![]
        }

        // Alphabet jumping in current column
        // Plain letter: jump to first item starting with that letter
        // Shift+letter: jump to first item where first char matches current item's first char
        //               AND second char matches the pressed letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let letter_lower = c.to_ascii_lowercase();
            let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.focused_mut() {
                    if use_second_char {
                        // Get the first letter of the currently selected item
                        let first_letter = col.items.get(col.selected_index)
                            .map(|item| item.title.chars().next())
                            .flatten()
                            .map(|ch| ch.to_ascii_lowercase());

                        if let Some(first_letter) = first_letter {
                            // Find first item starting with that letter AND having pressed letter as second char
                            if let Some(idx) = col.items.iter().position(|item| {
                                let mut chars = item.title.chars();
                                let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                                let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                                first == Some(first_letter) && second == Some(letter_lower)
                            }) {
                                col.selected_index = idx;
                            }
                        }
                    } else {
                        // Normal first-letter jump
                        if let Some(idx) = col.items.iter().position(|item| {
                            item.title.chars().next()
                                .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                                .unwrap_or(false)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                }
                // Clear columns to the right since selection changed
                folder_state.truncate_right_columns();
                folder_state.ensure_placeholder();
            }
            vec![]
        }

        _ => vec![],
    }
}

/// Handle Artist browsing with dynamic Miller columns.
pub(super) fn handle_artist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    let is_letter_jump = matches!(key.code, KeyCode::Char(c) if c.is_ascii_alphabetic())
        && !key.modifiers.contains(KeyModifiers::CONTROL);
    // Left/Backspace at root column → step into the alphabet strip
    // when it's visible (cat → strip → artists in either direction);
    // otherwise return straight to the category column.
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.artist_nav.can_go_left() {
        if state.alphabet_strip_visible() {
            state.alphabet_strip_focused = true;
        } else {
            state.focus_category_column();
        }
        return vec![];
    }

    let had_child = had_open_dependent(state, &state.artist_nav);
    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.artist_nav) {
        // Auto-drill: if a child column / track-pane was already open
        // when the user pressed Up/Down, re-target it at the new
        // selection without stealing focus. Same content update as a
        // click would do.
        if (is_up_down || is_letter_jump) && had_child {
            note_motion_for_lazy_art(state);
            let mut drill = drill_actions_for_focused_artist_item(state, false, true);
            actions.append(&mut drill);
            return actions;
        }
        // No dependent open → just selection change; truncate stale
        // children so the column stack stays coherent.
        if is_up_down || is_letter_jump {
            state.artist_nav.truncate_right();
        }
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill down into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        let allow_radio = key.code == KeyCode::Enter;
        return drill_actions_for_focused_artist_item(state, allow_radio, false);
    }

    vec![]
}

/// Mark that a rapid-navigation gesture just happened — raises the
/// lazy-art gate so artwork fetches are deferred until the user pauses
/// for `ART_LOAD_PAUSE_MS`. Both front-ends' settle ticks reopen the
/// gate and dispatch one batched fetch for the visible viewport.
fn note_motion_for_lazy_art(state: &mut AppState) {
    state.artwork.suppress_loads = true;
    state.artwork.last_motion_at = Some(std::time::Instant::now());
}

/// Whether a Miller child column is open to the right of the focused
/// column. This is the trigger for keyboard auto-drill on Up/Down:
/// when true at the start of the keypress, the handler re-targets
/// that child at the newly highlighted row instead of truncating it.
///
/// The track-details pane is deliberately *not* checked here. The
/// pane is a derived view of `focused_track()` and re-renders itself
/// every frame from the current selection, so arrowing through a
/// tracks column updates the pane automatically — no auto-drill
/// action needed, and emitting one would wrongly steal focus into
/// the pane.
fn had_open_dependent(
    _state: &AppState,
    nav: &crate::app::state::BrowseNavigationState,
) -> bool {
    nav.columns.len() > nav.focused_column + 1
}

/// Action(s) that drilling into the currently-selected artist_nav row
/// should produce. Used by both the Enter/Right handler AND the
/// auto-drill-on-selection-change path that fires when a child column
/// or the track-details pane is already open. `allow_radio` is true
/// only for the Enter key — Right arrow / auto-drill skip ArtistRadio
/// rows because starting a radio is a side-effect the user didn't ask
/// for when they were just sweeping selection.
pub(super) fn drill_actions_for_focused_artist_item(
    state: &mut AppState,
    allow_radio: bool,
    replace_child: bool,
) -> Vec<Action> {
    use crate::app::state::BrowseItem;
    let Some(item) = state.artist_nav.selected_item().cloned() else { return vec![] };
    match item {
        BrowseItem::Artist { key, title, .. } => {
            state.library.selected_artist_name = title;
            vec![MillerAction::LoadArtistAlbumsForMiller { artist_key: key, replace_child }.into()]
        }
        BrowseItem::Album { key, title, .. } => {
            state.library.selected_album_title = title;
            vec![MillerAction::LoadAlbumTracksForMiller { album_key: key, replace_child }.into()]
        }
        BrowseItem::AllArtists => vec![MillerAction::LoadAllAlbumsForMiller { replace_child }.into()],
        BrowseItem::ArtistRadio { artist_key, artist_name, .. } if allow_radio => {
            vec![RadioAction::StartPlexRadio { key: artist_key, title: artist_name }.into()]
        }
        BrowseItem::ArtistRadio { .. } => vec![],
        BrowseItem::AllTracks { scope, .. } => {
            use crate::app::state::AllTracksScope;
            match scope {
                AllTracksScope::Library => {
                    state.library.selected_album_title = "All Tracks".to_string();
                    vec![MillerAction::LoadAllLibraryTracksForMiller { replace_child }.into()]
                }
                AllTracksScope::AllCompilations => {
                    state.library.selected_album_title = "All Tracks".to_string();
                    vec![MillerAction::LoadAllCompilationTracksForMiller { replace_child }.into()]
                }
                AllTracksScope::CompilationsByArtist { artist_key, artist_name } => {
                    vec![MillerAction::LoadCompilationAllTracksForMiller {
                        artist_key,
                        artist_name,
                        replace_child,
                    }.into()]
                }
                AllTracksScope::Artist { artist_key, artist_name } => {
                    state.library.selected_album_title = format!("All tracks by {}", artist_name);
                    vec![MillerAction::LoadArtistAllTracksForMiller { artist_key, replace_child }.into()]
                }
            }
        }
        BrowseItem::Compilations => vec![MillerAction::LoadCompilationsForMiller { replace_child }.into()],
        BrowseItem::CompilationTracks { artist_key, artist_name } => {
            vec![MillerAction::LoadCompilationAlbumsForMiller { artist_key, artist_name, replace_child }.into()]
        }
        BrowseItem::Track { .. } => {
            // The pane is a passive viewer that follows the focused
            // track automatically. Auto-drill is a no-op (just keep
            // sweeping). On explicit Right/Enter:
            //   - pane closed → open it (focus stays on the tracks
            //     column; pane is just a side viewer)
            //   - pane already open → move focus *into* the pane so
            //     the user can use Up/Down to navigate similar
            //     tracks / Play. Right is the explicit "cross to
            //     the next column" gesture; with the pane already
            //     visible, that next column is the pane itself.
            if replace_child {
                vec![]
            } else if state.track_pane_open {
                state.track_pane_focused = true;
                state.track_pane_index = 0;
                state.category_column_focused = false;
                vec![]
            } else if state.focused_track().is_some() {
                vec![BrowseAction::OpenTrackDetails.into()]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Handle Genre browsing with dynamic Miller columns
pub(super) fn handle_genre_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Left/Backspace at root column → return to category column
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.tag_nav.can_go_left() {
        state.focus_category_column();
        return vec![];
    }

    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    let is_letter_jump = matches!(key.code, KeyCode::Char(c) if c.is_ascii_alphabetic())
        && !key.modifiers.contains(KeyModifiers::CONTROL);
    let had_child = had_open_dependent(state, &state.tag_nav);
    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.tag_nav) {
        if (is_up_down || is_letter_jump) && had_child {
            note_motion_for_lazy_art(state);
            let mut drill = drill_actions_for_focused_genre_item(state, true);
            actions.append(&mut drill);
            return actions;
        }
        if is_up_down || is_letter_jump {
            state.tag_nav.truncate_right();
        }
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        return drill_actions_for_focused_genre_item(state, false);
    }

    vec![]
}

/// Drill action for the currently-selected genre_nav row.
pub(super) fn drill_actions_for_focused_genre_item(
    state: &mut AppState,
    replace_child: bool,
) -> Vec<Action> {
    use crate::app::state::BrowseItem;
    let Some(item) = state.tag_nav.selected_item().cloned() else { return vec![] };
    match item {
        BrowseItem::GenreCategory { .. } => {
            // Legacy genre-tab UI is gone; tag sections drill straight
            // from a Tag/Genre item into albums.
            vec![]
        }
        BrowseItem::Genre { key, .. } => {
            vec![MillerAction::LoadGenreAlbumsForMiller { genre_key: key, replace_child }.into()]
        }
        BrowseItem::Album { key, title, .. } => {
            if let Some(col) = state.tag_nav.focused() {
                if col.grouped_by_album {
                    if let Some(new_col) = helpers::drill_grouped_album(col, col.selected_index) {
                        state.tag_nav.push_column(new_col);
                        return vec![];
                    }
                }
            }
            state.library.selected_album_title = title;
            vec![MillerAction::LoadGenreTracksForMiller { album_key: key, replace_child }.into()]
        }
        BrowseItem::Track { .. } => {
            // See artist drill helper — passive viewer; opens
            // without taking focus, second drill enters the pane.
            if replace_child {
                vec![]
            } else if state.track_pane_open {
                state.track_pane_focused = true;
                state.track_pane_index = 0;
                state.category_column_focused = false;
                vec![]
            } else if state.focused_track().is_some() {
                vec![BrowseAction::OpenTrackDetails.into()]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Handle Playlist browsing with dynamic Miller columns
pub(super) fn handle_playlist_browse_keys(key: event::KeyEvent, state: &mut AppState) -> Vec<Action> {
    // Left/Backspace at root column → return to category column
    if matches!(key.code, KeyCode::Left | KeyCode::Backspace) && !state.playlist_nav.can_go_left() {
        state.focus_category_column();
        return vec![];
    }

    // Handle common navigation keys
    let is_up_down = matches!(key.code, KeyCode::Up | KeyCode::Down);
    let is_letter_jump = matches!(key.code, KeyCode::Char(c) if c.is_ascii_alphabetic())
        && !key.modifiers.contains(KeyModifiers::CONTROL);
    let had_child = had_open_dependent(state, &state.playlist_nav);
    if let Some(mut actions) = handle_browse_nav_keys(key, &mut state.playlist_nav) {
        if (is_up_down || is_letter_jump) && had_child {
            note_motion_for_lazy_art(state);
            let mut drill = drill_actions_for_focused_playlist_item(state, true);
            actions.append(&mut drill);
            return actions;
        }
        if is_up_down || is_letter_jump {
            state.playlist_nav.truncate_right();
        }
        let art_batch = super::super::dispatch_miller::collect_viewport_art(state);
        if !art_batch.is_empty() {
            actions.push(SystemAction::LoadAlbumArt(art_batch).into());
        }
        return actions;
    }

    // Handle Enter/Right - drill into containers; Enter plays tracks
    if matches!(key.code, KeyCode::Enter | KeyCode::Right) {
        return drill_actions_for_focused_playlist_item(state, false);
    }

    vec![]
}

/// Drill action for the currently-selected playlist_nav row.
pub(super) fn drill_actions_for_focused_playlist_item(
    state: &mut AppState,
    replace_child: bool,
) -> Vec<Action> {
    use crate::app::state::BrowseItem;
    let Some(item) = state.playlist_nav.selected_item().cloned() else { return vec![] };
    match item {
        BrowseItem::Playlist { key, .. } => {
            vec![MillerAction::LoadPlaylistTracksForMiller { playlist_key: key, replace_child }.into()]
        }
        BrowseItem::Album { key, title, .. } => {
            if let Some(col) = state.playlist_nav.focused() {
                if col.grouped_by_album {
                    if let Some(new_col) = helpers::drill_grouped_album(col, col.selected_index) {
                        state.playlist_nav.push_column(new_col);
                        return vec![];
                    }
                }
            }
            state.library.selected_album_title = title;
            vec![MillerAction::LoadAlbumTracksForMiller { album_key: key, replace_child }.into()]
        }
        BrowseItem::Track { .. } => {
            // See artist drill helper — passive viewer; opens
            // without taking focus, second drill enters the pane.
            if replace_child {
                vec![]
            } else if state.track_pane_open {
                state.track_pane_focused = true;
                state.track_pane_index = 0;
                state.category_column_focused = false;
                vec![]
            } else if state.focused_track().is_some() {
                vec![BrowseAction::OpenTrackDetails.into()]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Returns Some(actions) if the key was handled, None if not.
pub fn handle_browse_nav_keys(
    key: event::KeyEvent,
    nav: &mut crate::app::state::BrowseNavigationState,
) -> Option<Vec<Action>> {
    match key.code {
        // Help
        KeyCode::F(1) | KeyCode::Char('?') => Some(vec![NavigationAction::SetView(View::Help).into()]),

        // Settings
        KeyCode::F(2) => Some(vec![SettingsAction::OpenSettings.into()]),

        // Up - move selection up (caller handles auto-drill or truncate)
        KeyCode::Up => {
            nav.move_up();
            Some(vec![])
        }

        // Down - move selection down (caller handles auto-drill or truncate)
        KeyCode::Down => {
            nav.move_down();
            Some(vec![])
        }

        // Page Up
        KeyCode::PageUp => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.selected_index.saturating_sub(10);
            }
            Some(vec![])
        }

        // Page Down
        KeyCode::PageDown => {
            if let Some(col) = nav.focused_mut() {
                let max_idx = col.items.len().saturating_sub(1);
                col.selected_index = (col.selected_index + 10).min(max_idx);
            }
            Some(vec![])
        }

        // Home - go to first item
        KeyCode::Home => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = 0;
            }
            Some(vec![])
        }

        // End - go to last item
        KeyCode::End => {
            if let Some(col) = nav.focused_mut() {
                col.selected_index = col.items.len().saturating_sub(1);
            }
            Some(vec![])
        }

        // Left/Backspace/Esc - focus previous column
        KeyCode::Left | KeyCode::Backspace | KeyCode::Esc => {
            if nav.can_go_left() {
                nav.focus_left();
            } else if key.code == KeyCode::Esc && nav.columns.len() > 1 {
                // At root column: Esc truncates child columns
                nav.columns.truncate(1);
            }
            Some(vec![])
        }

        // Tab is NOT handled here - it's handled globally to cycle between views
        // (Artists → Playlists → Genres → Folders → Now Playing)

        // Alphabet jumping in current column
        // Plain letter: jump to first item starting with that letter
        // Shift+letter: jump to first item where first char matches current item's first char
        //               AND second char matches the pressed letter
        KeyCode::Char(c) if c.is_ascii_alphabetic() && !key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Use the same `sort_key` the list is sorted by — strips
            // leading "The " etc., so "The Beatles" lives under "b" and
            // pressing `b` actually finds it. Without this, pressing
            // `b` searches for raw-title-starts-with-`b` and misses
            // every "The X" artist that's visually under B.
            use crate::app::handlers::helpers;
            if let Some(col) = nav.focused_mut() {
                let letter_lower = c.to_ascii_lowercase();
                let use_second_char = key.modifiers.contains(KeyModifiers::SHIFT);
                let sorted_first = |title: &str| -> Option<char> {
                    helpers::sort_key(title).chars().next()
                };

                if use_second_char {
                    // Anchor on the section letter (sort-key-based) of
                    // the currently selected item, then advance to the
                    // first item whose second sort-key char matches.
                    let first_letter = col.items.get(col.selected_index)
                        .and_then(|item| sorted_first(item.title()))
                        .map(|ch| ch.to_ascii_lowercase());

                    if let Some(first_letter) = first_letter {
                        if let Some(idx) = col.items.iter().position(|item| {
                            let key = helpers::sort_key(item.title());
                            let mut chars = key.chars();
                            let first = chars.next().map(|ch| ch.to_ascii_lowercase());
                            let second = chars.next().map(|ch| ch.to_ascii_lowercase());
                            first == Some(first_letter) && second == Some(letter_lower)
                        }) {
                            col.selected_index = idx;
                        }
                    }
                } else {
                    if let Some(idx) = col.items.iter().position(|item| {
                        sorted_first(item.title())
                            .map(|ch| ch.to_ascii_lowercase() == letter_lower)
                            .unwrap_or(false)
                    }) {
                        col.selected_index = idx;
                    }
                }
            }
            nav.truncate_right();
            Some(vec![])
        }

        // Not handled by common navigation
        _ => None,
    }
}

/// Update the column's selected_index for the filter's target category/column.
pub fn update_filter_column_selection(state: &mut AppState, item_idx: usize) {
    let category = state.list_filter.category;
    let column = state.list_filter.column;

    match category {
        BrowseCategory::Library => {
            if let Some(col) = state.artist_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
            }
        }
        BrowseCategory::Playlists => {
            if let Some(col) = state.playlist_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
            }
        }
        BrowseCategory::Folders => {
            if let Some(ref mut folder_state) = state.folder_state {
                if let Some(col) = folder_state.columns.get_mut(column) {
                    col.selected_index = item_idx;
                }
            }
        }
        cat if cat.is_tag_section() => {
            if let Some(col) = state.tag_nav.columns.get_mut(column) {
                col.selected_index = item_idx;
            }
        }
        _ => {}
    }
}

/// Truncate columns to the right of the filter's target column and move focus there.
/// Call this when the filter query or selection changes to remove stale drill-down columns.
pub fn truncate_filter_right_columns(state: &mut AppState) {
    let column = state.list_filter.column;

    match state.list_filter.category {
        BrowseCategory::Library => {
            state.artist_nav.columns.truncate(column + 1);
            state.artist_nav.focused_column = column;
        }
        BrowseCategory::Playlists => {
            state.playlist_nav.columns.truncate(column + 1);
            state.playlist_nav.focused_column = column;
        }
        BrowseCategory::Folders => {
            if let Some(ref mut fs) = state.folder_state {
                fs.columns.truncate(column + 1);
                fs.focused_column = column;
            }
        }
        cat if cat.is_tag_section() => {
            state.tag_nav.columns.truncate(column + 1);
            state.tag_nav.focused_column = column;
        }
        _ => {}
    }
}

/// Get the drill-down actions for the selected filtered item.
/// This simulates pressing Enter on the selected item to drill into it.
pub fn get_filter_drilldown_actions(state: &mut AppState) -> Vec<Action> {
    let category = state.list_filter.category;

    // Get the appropriate drill-down action based on category
    match category {
        BrowseCategory::Library => {
            // Use the artist_nav enter key logic
            handle_artist_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        BrowseCategory::Playlists => {
            handle_playlist_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        cat if cat.is_tag_section() => {
            handle_genre_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        BrowseCategory::Folders => {
            handle_folder_browse_keys(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                ),
                state,
            )
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AppState, BrowseCategory, BrowseColumn, BrowseItem, BrowseNavigationState};
    use crate::services::{FolderColumn, FolderNavigationState};

    fn make_browse_column(title: &str) -> BrowseColumn {
        BrowseColumn::new(title, vec![
            BrowseItem::Artist { key: "1".into(), title: "A".into(), thumb: None, is_placeholder: false },
        ])
    }

    fn make_folder_column(title: &str) -> FolderColumn {
        FolderColumn::new(None, title.into(), vec![])
    }

    #[test]
    fn test_truncate_folders_removes_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Folders;
        state.list_filter.column = 0;

        // Simulate 3 columns (root + 2 drill-downs), focused on column 2
        let mut fs = FolderNavigationState::for_library("lib1".into());
        fs.columns = vec![
            make_folder_column("root"),
            make_folder_column("sub1"),
            make_folder_column("sub2"),
        ];
        fs.focused_column = 2;
        state.folder_state = Some(fs);

        truncate_filter_right_columns(&mut state);

        let fs = state.folder_state.as_ref().unwrap();
        assert_eq!(fs.columns.len(), 1, "should keep only the filter column");
        assert_eq!(fs.focused_column, 0, "focus should move to filter column");
    }

    #[test]
    fn test_truncate_artists_removes_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Library;
        state.list_filter.column = 0;

        state.artist_nav = BrowseNavigationState {
            columns: vec![
                make_browse_column("artists"),
                make_browse_column("albums"),
                make_browse_column("tracks"),
            ],
            focused_column: 2,
            loading: false,
        };

        truncate_filter_right_columns(&mut state);

        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.focused_column, 0);
    }

    #[test]
    fn test_truncate_preserves_filter_column() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Folders;
        state.list_filter.column = 1; // filter on second column

        let mut fs = FolderNavigationState::for_library("lib1".into());
        fs.columns = vec![
            make_folder_column("root"),
            make_folder_column("sub1"),
            make_folder_column("sub2"),
        ];
        fs.focused_column = 2;
        state.folder_state = Some(fs);

        truncate_filter_right_columns(&mut state);

        let fs = state.folder_state.as_ref().unwrap();
        assert_eq!(fs.columns.len(), 2, "should keep root + filter column");
        assert_eq!(fs.focused_column, 1, "focus should move to filter column");
    }

    #[test]
    fn test_truncate_noop_when_no_right_columns() {
        let mut state = AppState::new();
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Library;
        state.list_filter.column = 0;

        state.artist_nav = BrowseNavigationState {
            columns: vec![make_browse_column("artists")],
            focused_column: 0,
            loading: false,
        };

        truncate_filter_right_columns(&mut state);

        assert_eq!(state.artist_nav.columns.len(), 1);
        assert_eq!(state.artist_nav.focused_column, 0);
    }
}

