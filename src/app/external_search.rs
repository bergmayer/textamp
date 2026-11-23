//! Shared external-search policy; each host owns its application handoff.
use super::{
    AppState,
    state::{
        BrowseColumn, BrowseDetails, BrowseItem, PaletteCommandKind, PaletteEntry, PlayAllRow, View,
    },
};
use crate::library::models::Track;
use crate::services::external_search::SearchTarget;

/// A validated search ready for the platform adapter.
#[derive(Debug)]
pub struct Request {
    /// Service that receives the query.
    pub target: SearchTarget,
    /// User text, kept separate from executable or URL syntax.
    pub query: String,
}

/// Why an external search cannot be dispatched.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} search is disabled in Settings")]
    Disabled(&'static str),
    #[error("No artist, album or track to search")]
    Empty,
}

pub(super) enum Subject<'a> {
    Artist(&'a str),
    Album { key: &'a str, title: &'a str },
    Track(&'a Track),
}

fn column_subject(column: &BrowseColumn) -> Option<Subject<'_>> {
    match &column.play_all_row {
        Some(PlayAllRow::ArtistRadio { artist_name, .. }) => Some(Subject::Artist(artist_name)),
        Some(PlayAllRow::Album { rating_key, title }) => Some(Subject::Album {
            key: rating_key,
            title,
        }),
        _ => match column.selected_item()? {
            BrowseItem::Artist {
                title,
                is_placeholder: false,
                ..
            } => Some(Subject::Artist(title)),
            BrowseItem::Album {
                key,
                title,
                is_placeholder: false,
                ..
            } => Some(Subject::Album { key, title }),
            BrowseItem::ArtistInfo { artist_name, .. } => Some(Subject::Artist(artist_name)),
            _ => match column.items.first() {
                Some(BrowseItem::ArtistInfo { artist_name, .. }) => {
                    Some(Subject::Artist(artist_name))
                }
                _ => None,
            },
        },
    }
}

pub(super) fn subject(state: &AppState) -> Option<Subject<'_>> {
    if state.view == View::Browse && !state.category_column_focused {
        if let Some((key, title)) = super::details::album_info_selection(state) {
            return Some(Subject::Album { key, title });
        }
        if let Some(track) = state
            .palette_track_list()
            .and_then(|(tracks, index)| tracks.get(index))
        {
            return Some(Subject::Track(track));
        }
        if let Some(nav) = state.browse_nav() {
            for column in nav.columns.iter().rev() {
                match &column.details {
                    Some(BrowseDetails::Artist(info)) => {
                        return Some(Subject::Artist(&info.artist_name));
                    }
                    Some(BrowseDetails::Album { .. }) => {
                        if let Some(subject) = column_subject(column) {
                            return Some(subject);
                        }
                    }
                    None => {}
                }
            }
            if let Some(subject) = nav.focused().and_then(column_subject) {
                return Some(subject);
            }
        }
    }
    state
        .palette_track_list()
        .and_then(|(tracks, index)| tracks.get(index))
        .or_else(|| state.current_track())
        .map(Subject::Track)
}

fn with_artist(artist: &str, title: &str) -> String {
    let artist = artist.trim();
    let title = title.trim();
    if artist.is_empty() {
        title.into()
    } else {
        format!("{artist} - {title}")
    }
}

/// Search text for an explicitly targeted track, including its individual performer.
pub fn track_query(track: &Track) -> String {
    with_artist(track.track_artist(), &track.title)
}

fn album_artist<'a>(state: &'a AppState, key: &str) -> &'a str {
    if let Some(nav) = state.browse_nav() {
        for column in nav.columns.iter().rev() {
            if let Some(BrowseItem::Album {
                key: album_key,
                artist,
                ..
            }) = column.selected_item()
                && album_key == key
            {
                return artist;
            }
            if let Some(track) = column
                .tracks
                .iter()
                .find(|track| track.parent_rating_key.as_deref() == Some(key))
            {
                return track
                    .grandparent_title
                    .as_deref()
                    .unwrap_or_else(|| track.artist_name());
            }
        }
    }
    state
        .library
        .albums
        .iter()
        .chain(&state.library.selected_artist_albums)
        .find(|album| album.rating_key == key)
        .map_or("", |album| album.artist_name())
}

/// Selection and focused details own the search before the surrounding column or playback.
pub fn query(state: &AppState) -> Option<String> {
    subject(state)
        .map(|subject| match subject {
            Subject::Artist(name) => name.to_owned(),
            Subject::Album { key, title } => with_artist(album_artist(state, key), title),
            Subject::Track(track) => track_query(track),
        })
        .filter(|query| !query.trim().is_empty())
}

/// Why a contextual search is unavailable, without allocating a query during rendering.
pub fn disabled_reason(state: &AppState, target: SearchTarget) -> Option<&'static str> {
    if !enabled(state, target) {
        return Some("Search service is disabled in Settings");
    }
    subject(state)
        .is_none()
        .then_some("No artist, album or track to search")
}

/// Service associated with a contextual palette command.
pub fn command_target(command: &PaletteCommandKind) -> Option<SearchTarget> {
    match command {
        PaletteCommandKind::SearchAppleMusic => Some(SearchTarget::AppleMusic),
        PaletteCommandKind::SearchSpotify => Some(SearchTarget::Spotify),
        PaletteCommandKind::SearchYouTube => Some(SearchTarget::YouTube),
        _ => None,
    }
}

/// Canonical labels and actions for external searches.
pub fn entry(target: SearchTarget) -> PaletteEntry {
    PaletteEntry {
        label: format!("Search {}", target.name()),
        hint: String::new(),
        command: match target {
            SearchTarget::AppleMusic => PaletteCommandKind::SearchAppleMusic,
            SearchTarget::Spotify => PaletteCommandKind::SearchSpotify,
            SearchTarget::YouTube => PaletteCommandKind::SearchYouTube,
        },
        aliases: match target {
            SearchTarget::AppleMusic => vec!["itunes".into()],
            SearchTarget::YouTube => vec!["yt".into()],
            SearchTarget::Spotify => vec![],
        },
    }
}

/// Whether a search service is enabled in the shared preferences.
pub fn enabled(state: &AppState, target: SearchTarget) -> bool {
    match target {
        SearchTarget::AppleMusic => state.external_search.apple_music,
        SearchTarget::Spotify => state.external_search.spotify,
        SearchTarget::YouTube => state.external_search.youtube,
    }
}

/// Resolve an explicit query or the current music selection.
/// # Errors
/// Rejects disabled services and empty queries.
pub fn request(
    state: &AppState,
    target: SearchTarget,
    query: Option<String>,
) -> Result<Request, Error> {
    if !enabled(state, target) {
        return Err(Error::Disabled(target.name()));
    }
    let query = query.or_else(|| self::query(state)).ok_or(Error::Empty)?;
    if query.trim().is_empty() {
        return Err(Error::Empty);
    }
    Ok(Request { target, query })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browsing() -> AppState {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.category_column_focused = false;
        state
    }
    fn track(key: &str, title: &str) -> Track {
        Track {
            rating_key: key.into(),
            title: title.into(),
            parent_rating_key: Some("album".into()),
            parent_title: Some("Voyager".into()),
            grandparent_title: Some("Momus".into()),
            ..Default::default()
        }
    }
    fn album_column() -> BrowseColumn {
        let tracks = vec![track("track", "Spacewalk")];
        let mut column =
            BrowseColumn::new_with_tracks("Voyager", BrowseItem::from_tracks(&tracks), tracks);
        column.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Voyager".into(),
        });
        column
    }
    #[test]
    fn selected_tracks_and_focused_details_take_priority_over_playback() {
        let mut state = browsing();
        state.queue.tracks = vec![track("playing", "Now Playing Song")];
        state.queue.index = Some(0);
        state.artist_nav.columns = vec![album_column()];
        assert_eq!(query(&state).as_deref(), Some("Momus - Spacewalk"));
        state.track_pane_open = true;
        assert_eq!(query(&state).as_deref(), Some("Momus - Spacewalk"));
        state.artist_nav.columns[0].header_selection =
            super::super::state::ColumnHeaderSelection::AlbumDetails;
        assert_eq!(query(&state).as_deref(), Some("Momus - Voyager"));
        state.artist_nav.columns[0].header_selection =
            super::super::state::ColumnHeaderSelection::Items;
        state
            .track_pane_similar
            .insert("track".into(), Ok(vec![track("similar", "Recommendation")]));
        state.track_pane_index = 3;
        state.track_pane_focused = true;
        assert_eq!(query(&state).as_deref(), Some("Momus - Recommendation"));
        state.sonic_details = Some(super::super::sonic_details::State {
            parent_key: "track".into(),
            tracks: vec![track("similar", "Recommendation")],
            index: 0,
            details: Default::default(),
            scroll_pin: None,
        });
        assert_eq!(query(&state).as_deref(), Some("Momus - Recommendation"));
        state.view = View::NowPlaying;
        assert_eq!(query(&state).as_deref(), Some("Momus - Now Playing Song"));
        state.queue.index = None;
        assert_eq!(query(&state).as_deref(), Some("Momus - Now Playing Song"));
        state.queue.tracks.clear();
        assert!(query(&state).is_none());
    }
    #[test]
    fn visible_artist_details_win_even_when_the_parent_column_has_focus() {
        use crate::app::state::ArtistInfoDetails;
        let mut state = browsing();
        let parent = BrowseColumn::new(
            "Artists",
            vec![BrowseItem::Artist {
                key: "other".into(),
                title: "Other Artist".into(),
                thumb: None,
                is_placeholder: false,
            }],
        );
        let mut details = BrowseColumn::new("Momus", vec![]);
        details.details = Some(BrowseDetails::Artist(ArtistInfoDetails {
            artist_key: "momus".into(),
            artist_name: "Momus".into(),
            request_id: 0,
            document: Default::default(),
            image_index: 0,
            task: None,
        }));
        state.artist_nav.columns = vec![parent, details];
        assert_eq!(query(&state).as_deref(), Some("Momus"));
        state.artist_nav.columns.pop();
        assert_eq!(query(&state).as_deref(), Some("Other Artist"));
        state.category_column_focused = true;
        assert!(query(&state).is_none());
    }
    #[test]
    fn search_without_a_subject_stays_visible_but_cannot_execute() {
        let mut state = browsing();
        state.artist_nav.columns = vec![BrowseColumn::new("Artists", vec![BrowseItem::AllArtists])];
        let entries = super::super::command_palette::materialize_entries(&state);
        let searches: Vec<_> = entries
            .iter()
            .filter(|entry| command_target(&entry.command).is_some())
            .collect();
        assert_eq!(searches.len(), 2);
        for entry in searches {
            assert!(
                super::super::command_catalog::disabled_reason(&state, &entry.command).is_some()
            );
            assert!(
                super::super::command_palette::run(entry.command.clone(), &mut state).is_empty()
            );
        }
        state.queue.tracks = vec![track("playing", "Spacewalk")];
        state.queue.index = Some(0);
        assert!(disabled_reason(&state, SearchTarget::AppleMusic).is_none());
        let actions =
            super::super::command_palette::run(PaletteCommandKind::SearchAppleMusic, &mut state);
        assert!(
            matches!(actions.as_slice(), [super::super::Action::System(super::super::action::SystemAction::OpenExternalSearch { query: Some(query), .. })] if query == "Momus - Spacewalk")
        );
    }

    #[test]
    fn search_preferences_validate_before_the_platform_handoff() {
        let mut state = AppState::new();
        let query = "Björk & 日本語 / music";
        let request = request(&state, SearchTarget::YouTube, Some(query.into())).unwrap();
        assert_eq!(request.query, query);
        assert!(matches!(
            super::request(&state, SearchTarget::AppleMusic, Some("  ".into())),
            Err(Error::Empty)
        ));
        assert!(matches!(
            super::request(&state, SearchTarget::Spotify, Some(query.into())),
            Err(Error::Disabled("Spotify"))
        ));
        state.external_search.spotify = true;
        assert!(super::request(&state, SearchTarget::Spotify, Some(query.into())).is_ok());
        let config: crate::config::UiConfig =
            toml::from_str("enable_spotify_search = true").unwrap();
        assert!(config.enable_spotify_search);
    }
}
