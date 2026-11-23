//! Shared Journey choices and explicit endpoint selection.
use super::{Action, action::SearchAction, state::*};

/// Keyboard focus in the terminal projection of the Journey form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum JourneyControl {
    #[default]
    Start,
    End,
    Count,
    Types,
    Results,
    Swap,
    Cancel,
    Preview,
}

/// A catalog choice displayed by either frontend's endpoint search.
pub enum Choice<'a> {
    Artist(&'a crate::library::models::Artist),
    Album(&'a crate::library::models::Album),
    Track(&'a crate::library::models::Track),
}
impl Choice<'_> {
    /// Provider-scoped identity retained when a choice is selected.
    pub fn key(&self) -> &str {
        match self {
            Self::Artist(a) => &a.rating_key,
            Self::Album(a) => &a.rating_key,
            Self::Track(t) => &t.rating_key,
        }
    }
    /// Primary label borrowed from the catalog item.
    pub fn title(&self) -> &str {
        match self {
            Self::Artist(a) => &a.title,
            Self::Album(a) => &a.title,
            Self::Track(t) => &t.title,
        }
    }
    /// Artist credit or item type used to distinguish equal titles.
    pub fn subtitle(&self) -> &str {
        match self {
            Self::Artist(_) => "Artist",
            Self::Album(a) => a.artist_name(),
            Self::Track(t) => t.track_artist(),
        }
    }
}

/// Endpoint results in the same index order used by selection and source requests.
pub fn choices(launcher: &AdventureLauncherState) -> Vec<Choice<'_>> {
    match &launcher.drill {
        AdventureDrillLevel::Search => {
            let Some(results) = &launcher.results else {
                return vec![];
            };
            let mut rows = Vec::new();
            if matches!(launcher.search_tab, SearchTab::Global | SearchTab::Artists) {
                rows.extend(results.artists.iter().map(Choice::Artist));
            }
            if matches!(launcher.search_tab, SearchTab::Global | SearchTab::Albums) {
                rows.extend(results.albums.iter().map(Choice::Album));
            }
            if matches!(launcher.search_tab, SearchTab::Global | SearchTab::Tracks) {
                rows.extend(results.tracks.iter().map(Choice::Track));
            }
            rows
        }
        AdventureDrillLevel::ArtistAlbums { albums, .. } => {
            albums.iter().map(Choice::Album).collect()
        }
        AdventureDrillLevel::AlbumTracks { tracks, .. } => {
            tracks.iter().map(Choice::Track).collect()
        }
    }
}

/// Drill into an artist/album or commit a track to the active endpoint.
pub fn select(launcher: &AdventureLauncherState) -> Vec<Action> {
    let rows = choices(launcher);
    let Some(choice) = rows.get(launcher.item_index) else {
        return vec![];
    };
    vec![
        match choice {
            Choice::Artist(artist) => SearchAction::AdventureLauncherDrillArtist {
                key: artist.rating_key.clone(),
                name: artist.title.clone(),
            },
            Choice::Album(album) => SearchAction::AdventureLauncherDrillAlbum {
                key: album.rating_key.clone(),
                title: album.title.clone(),
                artist_name: album.artist_name().into(),
            },
            Choice::Track(_) => SearchAction::AdventureLauncherSelectTrack,
        }
        .into(),
    ]
}

/// Form traversal excludes fields that do not apply to a seeded radio station.
pub fn controls(purpose: TrackPickerPurpose) -> &'static [JourneyControl] {
    use JourneyControl::*;
    if purpose == TrackPickerPurpose::Journey {
        &[Start, End, Count, Types, Results, Swap, Cancel, Preview]
    } else {
        &[Start, Types, Results, Cancel, Preview]
    }
}

/// Canonical labels for the endpoint-search form.
pub const START_TRACK: &str = "Start track";
pub const END_TRACK: &str = "End track";
pub const MAXIMUM_TRACKS: &str = "Maximum tracks (5–100)";
pub const PREVIEW: &str = "Preview Journey";
pub const PLAY: &str = "Play Journey";
pub const SWAP: &str = "Swap Tracks";

/// Invalid Journey length, shared by form validation and generation.
#[derive(Debug, thiserror::Error)]
#[error("Choose a maximum length from 5 to 100 tracks.")]
pub struct InvalidTrackCount;

/// Parse and validate the same maximum length for every Journey frontend.
/// # Errors
/// Returns an error unless the input is an integer from 5 through 100.
pub fn track_count(input: &str) -> Result<usize, InvalidTrackCount> {
    input
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|count| (5..=100).contains(count))
        .ok_or(InvalidTrackCount)
}
