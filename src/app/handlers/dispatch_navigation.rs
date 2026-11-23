//! Navigation dispatch handlers: SetView, NextView, PrevView, NextMode, PrevMode,
//! SetCategory, ToggleFocus.

use crate::app::action::{BrowseAction, FolderAction, NavigationAction, SystemAction};
use crate::app::state::{BrowseCategory, Focus, RightPanelMode, View};
use crate::app::{Action, AppState, Event};

use anyhow::Result;
use tokio::sync::mpsc;

/// Dispatch navigation actions. Returns follow-up actions.
pub fn dispatch(
    _event_tx: &mpsc::Sender<Event>,
    action: NavigationAction,
    state: &mut AppState,
) -> Result<Vec<Action>> {
    let mut follow_ups = vec![];

    // Deactivate inline filter on view or category change
    if matches!(
        action,
        NavigationAction::SetView(_) | NavigationAction::SetCategory { .. }
    ) && state.list_filter.active
    {
        state.list_filter.deactivate();
    }

    match action {
        NavigationAction::SwitchPresentation(mode) => {
            if state.windowed_presentation.can_activate(mode) {
                state.windowed_presentation.request(mode);
            } else {
                state.set_status("This presentation is unavailable in the current host".into());
            }
        }
        NavigationAction::OpenRadioStation(key) => {
            return Ok(crate::app::radio_navigation::configure(state, &key));
        }
        NavigationAction::OpenLibraryRoot(root) => {
            return Ok(crate::app::sidebar::open_root(root, state));
        }
        NavigationAction::ToggleSidebar => {
            state.tui_sidebar_visible = !state.tui_sidebar_visible;
            if !state.tui_sidebar_visible {
                state.category_column_focused = false;
            }
        }
        NavigationAction::ToggleQueuePanel => {
            state.tui_queue_visible = !state.tui_queue_visible;
            if !state.tui_queue_visible && state.view == View::Queue {
                state.set_view(state.tui_queue_workspace.view());
            }
        }

        NavigationAction::Back => return Ok(crate::app::navigation_history::back(state)),
        NavigationAction::Undo => return Ok(crate::app::navigation_history::undo(state)),
        NavigationAction::ResetDesktopPanels => {}

        NavigationAction::DesktopFullscreen(_) => {}
        NavigationAction::ToggleFullscreen | NavigationAction::FullscreenArtwork => {
            use crate::app::state::FullscreenView;
            state.fullscreen = match (action, state.fullscreen) {
                (NavigationAction::FullscreenArtwork, _) => FullscreenView::Artwork,
                (_, FullscreenView::Closed) => FullscreenView::Visualizer,
                _ => FullscreenView::Closed,
            };
            if state.fullscreen.is_open() {
                state.popups.close_all();
                state.palette.close();
                state.seek_drag = None;
                state.volume_drag = false;
                state.scroll.scrollbar_drag = None;
                state.miller_h_drag_grab = None;
                if !state.fullscreen.is_artwork() {
                    follow_ups.push(SystemAction::LoadWaveform.into());
                    follow_ups.push(SystemAction::LoadSpectrogram.into());
                }
            }
        }
        NavigationAction::SetView(view) => {
            state.set_view(view);
            if view == View::Queue {
                state.tui_queue_visible = true;
            }
            // Load stations when entering Queue view if not already loaded
            if view == View::Queue
                && state.station_nav.columns.is_empty()
                && !state.station_nav.loading
            {
                follow_ups.push(BrowseAction::LoadStations.into());
            }
            // Load waveform and spectrogram when entering NowPlaying view
            if view == View::NowPlaying {
                follow_ups.push(SystemAction::LoadWaveform.into());
                follow_ups.push(SystemAction::LoadSpectrogram.into());
            }
        }
        NavigationAction::NextView => {
            // Tab: cycle through top-level views
            // Order: Browse → Queue → Now Playing → Browse
            if state.view == View::NowPlaying {
                state.set_view(View::Browse);
            } else if state.view == View::Queue {
                state.set_view(View::NowPlaying);
            } else if state.view == View::Browse {
                state.set_view(View::Queue);
            } else {
                state.set_view(View::Browse);
            }
        }
        NavigationAction::PrevView => {
            // Shift+Tab: cycle backwards through top-level views
            // Order: Browse ← Queue ← Now Playing ← Browse
            if state.view == View::NowPlaying {
                state.set_view(View::Queue);
            } else if state.view == View::Queue {
                state.set_view(View::Browse);
            } else if state.view == View::Browse {
                state.set_view(View::NowPlaying);
            } else {
                state.set_view(View::Browse);
            }
        }
        NavigationAction::SetCategory {
            category,
            preserve_sections_focus,
        } => {
            if category == BrowseCategory::Library
                && matches!(
                    state.desktop_library_root,
                    crate::app::state::LibraryRoot::Singles
                        | crate::app::state::LibraryRoot::Mp3Players
                )
            {
                state.desktop_library_root = crate::app::state::LibraryRoot::Artists;
                state
                    .artist_nav
                    .reset("artists", state.build_artist_root_items());
            }
            // Picking a category implies "show me that part of the
            // library", so always switch to the Browse view too.
            // Without this, the View menu / Ctrl+L|P|G|O shortcuts
            // would silently change the category state but leave the
            // user stuck on Queue/Now Playing/Help/etc.
            if state.view != View::Browse || state.fullscreen.is_open() {
                state.set_view(View::Browse);
            }

            // `preserve_sections_focus` is true on a sections-column
            // arrow-key sweep — the user is still navigating with the
            // keyboard in the sections column, the rightward content
            // should follow without focus stealing. Click / Enter /
            // palette / shortcut paths set it to false so the action
            // transfers focus to the new category's content.
            if !preserve_sections_focus {
                state.category_column_focused = false;
            }

            if state.browse_category != category {
                // Unshuffle Library root when leaving, so "All Artists" is in correct position
                if state.browse_category == BrowseCategory::Library
                    && let Some(col) = state.artist_nav.columns.first_mut()
                    && col.is_shuffled()
                {
                    col.unshuffle();
                }
                state.set_browse_category(category, preserve_sections_focus);
                state.focus = Focus::Left;
                // Clear right panel
                state.library.right_panel_mode = RightPanelMode::Empty;
                state.library.selected_artist_albums.clear();
                state.library.selected_album_tracks.clear();

                // Load category data if needed (and not already loading)
                match category {
                    BrowseCategory::Library
                        if state.library.artists.is_empty() && !state.library.artists_loading =>
                    {
                        return Ok(vec![crate::app::action::DataAction::LoadArtists.into()]);
                    }
                    BrowseCategory::Playlists => {
                        if state.library.playlists.is_empty() && !state.library.playlists_loading {
                            return Ok(vec![crate::app::action::DataAction::LoadPlaylists.into()]);
                        } else {
                            // Rebuild root column from state.library.playlists to ensure it's populated
                            // (guards against stale/empty nav from preload race conditions)
                            let items = crate::app::state::BrowseItem::from_playlists(
                                &state.library.playlists,
                            );
                            state.playlist_nav.reset("playlists", items);
                        }
                    }
                    BrowseCategory::Folders if state.folder_state.is_none() => {
                        follow_ups.push(FolderAction::LoadFolderRoot.into());
                    }
                    cat if cat.is_tag_section() => {
                        // Reset shared tag_nav for the new section.
                        state.tag_nav.columns.clear();
                        state.tag_nav.focused_column = 0;
                        follow_ups.push(BrowseAction::RefreshTagView.into());
                    }
                    _ => {}
                }
            }
        }
        NavigationAction::ToggleFocus => {
            state.focus = match state.focus {
                Focus::Left => Focus::Right,
                Focus::Right => Focus::Left,
            };
        }
    }
    Ok(follow_ups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>, AppState) {
        let (tx, rx) = mpsc::channel(100);
        let state = AppState::new();
        (tx, rx, state)
    }

    #[test]
    fn toggle_focus_left_to_right() {
        let (tx, _rx, mut state) = setup();
        state.focus = Focus::Left;

        dispatch(&tx, NavigationAction::ToggleFocus, &mut state).unwrap();
        assert_eq!(state.focus, Focus::Right);
    }

    #[test]
    fn toggle_focus_right_to_left() {
        let (tx, _rx, mut state) = setup();
        state.focus = Focus::Right;

        dispatch(&tx, NavigationAction::ToggleFocus, &mut state).unwrap();
        assert_eq!(state.focus, Focus::Left);
    }

    #[test]
    fn set_view_changes_state() {
        let (tx, _rx, mut state) = setup();

        dispatch(&tx, NavigationAction::SetView(View::Queue), &mut state).unwrap();
        assert_eq!(state.view, View::Queue);

        dispatch(&tx, NavigationAction::SetView(View::Help), &mut state).unwrap();
        assert_eq!(state.view, View::Help);
    }

    #[test]
    fn set_view_queue_requests_load_stations() {
        let (tx, _rx, mut state) = setup();

        let follow_ups = dispatch(&tx, NavigationAction::SetView(View::Queue), &mut state).unwrap();
        assert!(
            follow_ups
                .iter()
                .any(|a| matches!(a, Action::Browse(BrowseAction::LoadStations)))
        );
    }

    #[test]
    fn set_view_now_playing_requests_waveform() {
        let (tx, _rx, mut state) = setup();

        let follow_ups =
            dispatch(&tx, NavigationAction::SetView(View::NowPlaying), &mut state).unwrap();
        assert!(
            follow_ups
                .iter()
                .any(|a| matches!(a, Action::System(SystemAction::LoadWaveform)))
        );
        assert!(
            follow_ups
                .iter()
                .any(|a| matches!(a, Action::System(SystemAction::LoadSpectrogram)))
        );
    }

    #[test]
    fn next_view_browse_to_queue() {
        let (tx, _rx, mut state) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library, false);

        dispatch(&tx, NavigationAction::NextView, &mut state).unwrap();
        assert_eq!(state.view, View::Queue);
    }

    #[test]
    fn next_view_queue_to_now_playing() {
        let (tx, _rx, mut state) = setup();
        state.view = View::Queue;

        dispatch(&tx, NavigationAction::NextView, &mut state).unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[test]
    fn next_view_now_playing_to_browse() {
        let (tx, _rx, mut state) = setup();
        state.view = View::NowPlaying;

        dispatch(&tx, NavigationAction::NextView, &mut state).unwrap();
        assert_eq!(state.view, View::Browse);
    }

    #[test]
    fn prev_view_library_to_now_playing() {
        let (tx, _rx, mut state) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library, false);

        dispatch(&tx, NavigationAction::PrevView, &mut state).unwrap();
        assert_eq!(state.view, View::NowPlaying);
    }

    #[test]
    fn prev_view_now_playing_to_queue() {
        let (tx, _rx, mut state) = setup();
        state.view = View::NowPlaying;

        dispatch(&tx, NavigationAction::PrevView, &mut state).unwrap();
        assert_eq!(state.view, View::Queue);
    }

    #[test]
    fn set_category_changes_and_resets_focus() {
        let (tx, _rx, mut state) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library, false);
        state.focus = Focus::Right;

        dispatch(
            &tx,
            NavigationAction::set_category(BrowseCategory::AlbumGenres),
            &mut state,
        )
        .unwrap();
        assert_eq!(state.browse_category, BrowseCategory::AlbumGenres);
        assert_eq!(state.focus, Focus::Left);
    }

    #[test]
    fn set_category_same_is_noop() {
        let (tx, _rx, mut state) = setup();
        state.view = View::Browse;
        state.set_browse_category(BrowseCategory::Library, false);
        state.focus = Focus::Right; // keep right focus

        dispatch(
            &tx,
            NavigationAction::set_category(BrowseCategory::Library),
            &mut state,
        )
        .unwrap();
        // Category didn't change, so focus should remain Right
        assert_eq!(state.focus, Focus::Right);
    }
}
