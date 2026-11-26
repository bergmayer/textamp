//! Canonical detail kinds, related sections and metadata order.
use super::{
    AppState,
    action::*,
    playback_controls::{Control, album_controls, collection_controls, track_controls},
    state::*,
};
use crate::library::models::Track;
/// Domain identity of a library details surface.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Kind {
    Album,
    #[default]
    Track,
    Artist,
}
impl Kind {
    /// Canonical title used for navigation and details controls.
    pub fn label(self) -> &'static str {
        match self {
            Self::Album => "Album Details",
            Self::Track => "Track Details",
            Self::Artist => "Artist Details",
        }
    }
}
/// Metadata in display order, omitting unavailable values.
pub fn track_rows(track: &Track) -> Vec<(String, String)> {
    let mut rows = vec![
        ("Artist".into(), track.track_artist().into()),
        ("Album".into(), track.album_name().into()),
    ];
    if let Some(year) = track.year.or(track.parent_year) {
        rows.push(("Year".into(), year.to_string()));
    }
    if !track.genres.is_empty() {
        rows.push(("Genres".into(), track.genres.join(", ")));
    }
    if let Some(duration) = track.duration {
        rows.push(("Duration".into(), crate::util::format_duration(duration)));
    }
    if let Some(number) = track.index {
        rows.push(("Track".into(), number.to_string()));
    }
    if let Some(media) = track.media.first() {
        if let Some(bitrate) = media.bitrate {
            rows.push(("Bit rate".into(), format!("{bitrate} kbps")));
        }
        if let Some(channels) = media.audio_channels {
            rows.push(("Channels".into(), channels.to_string()));
        }
        if let Some(format) = media.container.as_deref().filter(|container| {
            media
                .audio_codec
                .as_deref()
                .is_some_and(|codec| codec.eq_ignore_ascii_case(container))
        }) {
            rows.push(("Format".into(), format.to_owned()));
        } else {
            if let Some(container) = &media.container {
                rows.push(("Container".into(), container.clone()));
            }
            if let Some(codec) = &media.audio_codec {
                rows.push(("Codec".into(), codec.clone()));
            }
        }
    }
    rows.retain(|(_, v)| !v.is_empty());
    rows
}

impl Kind {
    /// Related content follows information in every detail surface.
    pub fn related_label(self) -> &'static str {
        match self {
            Self::Track => "Similar Tracks",
            Self::Album => "Similar Albums",
            Self::Artist => "Related Artists",
        }
    }
}

/// Detail workflow content without frontend widgets or geometry.
#[derive(Clone, Default)]
pub struct Document {
    pub kind: Kind,
    pub artist_bio: Option<Control>,
    pub track_artwork: bool,
    pub lyrics: Option<String>,
    pub artist_artwork_visible: bool,
    pub title: String,
    pub subtitle: String,
    pub rows: Vec<(String, String)>,
    pub controls: Vec<Control>,
    pub similar: Option<Similar>,
    pub related_artists: Option<Vec<crate::library::models::Artist>>,
    pub text: String,
    pub album_info: Option<super::sources::navidrome::album_notes::AlbumInfo>,
    pub loading: bool,
    pub track: Option<Track>,
    pub genres: Vec<String>,
}
fn album_navigation_track(state: &AppState, key: &str) -> Option<Track> {
    if (state.downloads.is_offline() || super::downloads::is_root_column(state, 0))
        && let Some(entry) = state
            .downloads
            .store
            .entries
            .values()
            .find(|entry| super::downloads::album_key(&entry.track) == key)
    {
        return Some(entry.track.clone());
    }
    if let Some(album) = state
        .library
        .albums
        .iter()
        .chain(&state.library.selected_artist_albums)
        .find(|album| album.rating_key == key)
    {
        return Some(Track {
            parent_rating_key: Some(album.rating_key.clone()),
            parent_title: Some(album.title.clone()),
            parent_year: album.year,
            parent_thumb: album.thumb.clone(),
            grandparent_rating_key: album.parent_rating_key.clone(),
            grandparent_title: album.parent_title.clone(),
            original_title: album.parent_title.clone(),
            genres: album.genre.iter().map(|genre| genre.tag.clone()).collect(),
            ..Default::default()
        });
    }
    state
        .browse_nav()?
        .columns
        .iter()
        .flat_map(|column| &column.tracks)
        .find(|track| track.parent_rating_key.as_deref() == Some(key))
        .cloned()
}

/// Related music with its loading or failure message.
#[derive(Clone, Default)]
pub struct Similar {
    pub albums: bool,
    pub message: String,
    pub tracks: Vec<Track>,
}
/// The album page requests its cover independently of list-thumbnail preferences.
pub fn album_cover(state: &AppState, column: &BrowseColumn) -> Option<(String, String)> {
    let PlayAllRow::Album { rating_key, .. } = column.play_all_row.as_ref()? else {
        return None;
    };
    let thumb = column
        .tracks
        .iter()
        .find_map(|track| track.parent_thumb.clone())
        .or_else(|| {
            state
                .library
                .albums
                .iter()
                .find(|album| &album.rating_key == rating_key)
                .and_then(|album| album.thumb.clone())
        })
        .or_else(|| super::downloads::is_root_column(state, 0).then(String::new))?;
    Some((rating_key.clone(), thumb))
}

/// Selected album details and their catalog identity.
pub fn album_info_selection(state: &AppState) -> Option<(&str, &str)> {
    let column = state.browse_nav()?.focused()?;
    if state.view != View::Browse
        || state.sonic_details.is_some()
        || (state.track_pane_open && column.header_selection != ColumnHeaderSelection::AlbumDetails)
    {
        return None;
    }
    match &column.play_all_row {
        Some(PlayAllRow::Album { rating_key, title }) => Some((rating_key, title)),
        _ => None,
    }
}
fn album_info_preview(state: &AppState, key: &str, title: &str) -> Document {
    let track = album_navigation_track(state, key).or_else(|| {
        state.browse_nav()?.columns.iter().find(|c|
            matches!(&c.play_all_row, Some(PlayAllRow::Album { rating_key, .. }) if rating_key == key)
        )?.tracks.first().cloned()
    });
    let mut rows = vec![];
    if let Some(year) = track.as_ref().and_then(|t| t.parent_year.or(t.year)) {
        rows.push(("Year".into(), year.to_string()));
    }
    let genres = track.as_ref().map(|t| t.genres.clone()).unwrap_or_default();
    if !genres.is_empty() {
        rows.push(("Genres".into(), genres.join(", ")));
    }
    if let Some(column) = state.browse_nav().and_then(|nav| {
        nav.columns.iter().rev().find(|column| {
            !column.tracks.is_empty()
                && matches!(&column.play_all_row, Some(PlayAllRow::Album { rating_key, .. }) if rating_key == key)
        })
    }) {
        rows.push(("Tracks".into(), column.tracks.len().to_string()));
        let duration = column.tracks.iter().filter_map(|t| t.duration).sum();
        rows.push(("Duration".into(), crate::util::format_duration(duration)));
    }
    Document {
        album_info: state
            .sources
            .album_notes
            .as_ref()
            .filter(|n| n.matches(state.library_generation, key))
            .map(|n| n.info.clone()),
        kind: Kind::Album,
        title: title.into(),
        subtitle: track
            .as_ref()
            .map(|t| {
                t.grandparent_title
                    .as_deref()
                    .unwrap_or_else(|| t.artist_name())
                    .to_owned()
            })
            .unwrap_or_default(),
        rows,
        genres,
        track,
        controls: album_controls(key, title),
        ..Default::default()
    }
}
/// Resolve related-music ownership from the selected library destination.
pub fn similar_seed(state: &AppState) -> Option<(Track, bool)> {
    if !crate::app::sources::navidrome::recommendations::available(state) {
        return None;
    }
    if state.view == View::NowPlaying && state.now_playing_details_visible {
        return state.current_track().cloned().map(|track| (track, false));
    }
    if state.view != View::Browse || state.desktop_player_visible {
        return None;
    }
    if let Some(track) = crate::app::sonic_details::selected_track(state) {
        return Some((track.clone(), false));
    }
    let nav = state.browse_nav();
    let album = nav.and_then(|nav| {
        nav.focused()
            .filter(|c| c.has_album_details())
            .or_else(|| nav.columns.last().filter(|c| c.has_album_details()))
    });
    if (!state.track_pane_open || album_info_selection(state).is_some())
        && let Some(column) = album
    {
        return column.tracks.first().cloned().map(|track| (track, true));
    }
    if !state.track_pane_open {
        return None;
    }
    if state.browse_category == BrowseCategory::Folders {
        let column = state.folder_state.as_ref()?.focused()?;
        return column
            .items
            .get(column.selected_index)
            .and_then(|item| column.tracks.get(&item.key))
            .map(|track| (track.clone(), false));
    }
    if let Some(track) = state.focused_track() {
        return Some((track.clone(), false));
    }
    None
}
/// Related content distinguishes an owned request, a completed result and a retryable failure.
pub fn related_message(
    result: Option<&Result<Vec<Track>, String>>,
    pending: bool,
) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    if pending {
        return Some(Cow::Borrowed("Loading similar music…"));
    }
    match result {
        Some(Ok(tracks)) if !tracks.is_empty() => None,
        Some(Ok(_)) => Some(Cow::Borrowed("No similar music found.")),
        Some(Err(error)) => Some(Cow::Owned(format!(
            "{error} · Choose Refresh Library to retry."
        ))),
        None => Some(Cow::Borrowed("Similar music is unavailable.")),
    }
}

/// Keep a cached biography readable while its owned refresh is pending.
impl ArtistInfoDetails {
    pub fn needs_web_search(&self) -> bool {
        !self.document.has_prose()
    }
    pub fn excerpt(&self) -> &str {
        if self.needs_web_search() {
            return "";
        }
        let text = artist_text(self).trim();
        text.split_once('\n')
            .filter(|(heading, _)| heading.trim().eq_ignore_ascii_case(&self.artist_name))
            .map_or(text, |(_, body)| body.trim_start())
    }
}
pub fn artist_text(info: &ArtistInfoDetails) -> &str {
    if info.document.text.is_empty() && info.task.is_some() {
        "Loading artist details…"
    } else {
        &info.document.text
    }
}

/// Project the selected document and the commands available for it.
pub fn project(state: &AppState) -> Option<Document> {
    let mut preview = preview_details(state)?;
    if preview.kind == Kind::Track
        && let Some(track) = &preview.track
    {
        preview.track_artwork = track_cover(state, track).is_some();
        preview.lyrics = super::lyrics::inline_text(state, track).map(str::to_owned);
    }
    let download = match preview.kind {
        Kind::Album => album_info_selection(state).map(|(key, _)| {
            Control::new(
                "Download Album",
                super::downloads::Command::Album(key.into()),
            )
        }),
        Kind::Track => preview.track.as_ref().map(|track| {
            if let Some((id, _)) = state.downloads.store.find(track) {
                Control::new(
                    "Remove Download",
                    super::downloads::Command::Remove(id.into()),
                )
            } else {
                Control::new(
                    "Download",
                    super::downloads::Command::Tracks(vec![track.clone()]),
                )
            }
        }),
        Kind::Artist => None,
    };
    if let Some(control) = download {
        preview.controls.push(control);
    }
    Some(with_related(state, preview, similar_seed(state)))
}

/// Metadata, inline lyrics and recommendations for the current playback item.
pub fn now_playing_details(state: &AppState) -> Option<Document> {
    if state.view != View::NowPlaying || !state.now_playing_details_visible {
        return None;
    }
    let track = state.current_track()?;
    let doc = Document {
        title: track.title.clone(),
        subtitle: track.track_artist().into(),
        rows: now_playing_rows(track),
        lyrics: super::lyrics::inline_text(state, track).map(str::to_owned),
        track: Some(track.clone()),
        ..Default::default()
    };
    Some(with_related(state, doc, Some((track.clone(), false))))
}

/// Resolve the biography for the credited album artist.
pub fn album_artist(state: &AppState, column: &BrowseColumn) -> Option<(String, String)> {
    let PlayAllRow::Album { rating_key, .. } = column.play_all_row.as_ref()? else {
        return None;
    };
    let track =
        album_navigation_track(state, rating_key).or_else(|| column.tracks.first().cloned())?;
    let name = track
        .grandparent_title
        .as_deref()
        .filter(|name| !name.trim().is_empty())?;
    let artist_key = track.grandparent_rating_key.clone().or_else(|| {
        state
            .library
            .artists
            .iter()
            .find(|artist| artist.title.eq_ignore_ascii_case(name))
            .map(|artist| artist.rating_key.clone())
    })?;
    Some((artist_key, name.to_owned()))
}

pub fn album_bio(state: &AppState, column: &BrowseColumn) -> Option<Control> {
    let (artist_key, name) = album_artist(state, column)?;
    if !super::handlers::helpers::biography::known(state, &artist_key)
        .is_some_and(crate::services::biography::Biography::has_prose)
    {
        let provider = state.web_search_provider;
        let mut control = Control::new(
            provider.search_label(),
            SystemAction::OpenBrowser(provider.artist_url(&name)),
        );
        control.search_provider = Some(provider);
        return Some(control);
    }
    Some(Control::new(
        "Artist Biography",
        SearchAction::ShowArtistBio {
            artist_key,
            artist_name: name.to_owned(),
        },
    ))
}

/// Cover identifiers distinguish track art before both images are cached.
pub fn track_cover<'a>(state: &AppState, track: &'a Track) -> Option<&'a str> {
    let thumb = track.thumb.as_deref().filter(|thumb| !thumb.is_empty())?;
    let album_thumb = track.parent_thumb.as_deref().or_else(|| {
        state
            .library
            .albums
            .iter()
            .find(|album| Some(&album.rating_key) == track.parent_rating_key.as_ref())
            .and_then(|album| album.thumb.as_deref())
    });
    if album_thumb.is_none_or(|cover| cover == thumb)
        || Some(thumb) == track.parent_rating_key.as_deref()
    {
        return None;
    }
    let cached = &state.artwork.grid_cache;
    if let Some((track_image, album_image)) = cached.get(&track.rating_key).zip(
        track
            .parent_rating_key
            .as_ref()
            .and_then(|key| cached.get(key)),
    ) && track_image == album_image
    {
        return None;
    }
    Some(thumb)
}

/// Project the album owned by this column independently of keyboard focus.
pub fn album_page(state: &AppState, column: &BrowseColumn) -> Option<Document> {
    let PlayAllRow::Album { rating_key, title } = column.play_all_row.as_ref()? else {
        return None;
    };
    let mut page = album_info_preview(state, rating_key, title);
    page.artist_bio = album_bio(state, column);
    if let Some(related) = &state.sources.album_artists
        && related.generation == state.library_generation
        && related.album == *rating_key
    {
        page.related_artists = related
            .document
            .as_ref()
            .map(|doc| doc.related_artists.clone());
    }
    page.controls.push(Control::new(
        "Download Album",
        super::downloads::Command::Album(rating_key.clone()),
    ));
    Some(with_related(
        state,
        page,
        column.tracks.first().cloned().map(|track| (track, true)),
    ))
}

pub fn open_album_notes(state: &mut AppState, index: usize) {
    let Some(column) = state.browse_nav().and_then(|nav| nav.columns.get(index)) else {
        return;
    };
    let Some(PlayAllRow::Album { rating_key, title }) = &column.play_all_row else {
        return;
    };
    let info = state
        .sources
        .album_notes
        .as_ref()
        .filter(|notes| notes.matches(state.library_generation, rating_key));
    let mut text = info
        .map(|notes| {
            notes
                .info
                .review
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .unwrap_or_default();
    if let Some(url) = info.and_then(|notes| notes.info.display_link()) {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(url.as_str());
    }
    if text.is_empty() {
        text = "No album notes available.".into();
    }
    state.popups.set_text(Some(crate::app::state::TextPopup {
        album_notes: true,
        title: format!("Album Notes · {title}"),
        text,
        scroll: 0,
        request_id: 0,
        lyrics_query: None,
    }));
}

pub(crate) fn recommendation_tracks<'a>(
    seed: &'a Track,
    tracks: &'a [Track],
    albums: bool,
) -> impl Iterator<Item = &'a Track> {
    let mut seen = std::collections::HashSet::new();
    tracks
        .iter()
        .filter(move |track| !super::track_context::same_artist(seed, track))
        .filter(move |track| {
            !albums
                || (track.parent_rating_key != seed.parent_rating_key
                    && track
                        .parent_rating_key
                        .as_deref()
                        .is_some_and(|key| seen.insert(key)))
        })
        .take(12)
}

fn with_related(state: &AppState, mut preview: Document, seed: Option<(Track, bool)>) -> Document {
    if state.downloads.is_offline() {
        if preview.kind == Kind::Artist {
            return preview;
        }
        preview.similar = Some(Similar {
            message: "Unavailable offline".into(),
            ..Default::default()
        });
        return preview;
    }
    if crate::app::sources::navidrome::recommendations::available(state)
        && let Some((seed, albums)) = seed
    {
        let mut similar = Similar {
            albums,
            ..Default::default()
        };
        match state.track_pane_similar.get(&seed.rating_key) {
            Some(Ok(tracks)) if !state.track_pane_similar_loading.contains(&seed.rating_key) => {
                similar.tracks = recommendation_tracks(&seed, tracks, albums)
                    .cloned()
                    .collect();
                if similar.tracks.is_empty() {
                    similar.message = "No similar music from other artists found.".into();
                }
            }
            result => {
                similar.message = related_message(
                    result,
                    state.track_pane_similar_loading.contains(&seed.rating_key)
                        || (result.is_none()
                            && similar_seed(state)
                                .is_some_and(|(visible, _)| visible.rating_key == seed.rating_key)),
                )
                .map(|message| message.into_owned())
                .unwrap_or_default()
            }
        }
        preview.similar = Some(similar);
    }
    preview
}
fn preview_details(state: &AppState) -> Option<Document> {
    if let Some((key, title)) = album_info_selection(state) {
        return Some(album_info_preview(state, key, title));
    }
    if let Some(index) = state.artist_info_column_index()
        && let Some(BrowseDetails::Artist(info)) = state
            .browse_nav()
            .and_then(|nav| nav.columns.get(index))
            .and_then(|column| column.details.as_ref())
    {
        let controls = artist_commands(info)
            .into_iter()
            .map(|command| {
                let label = if command == ArtistCommand::SearchWeb {
                    state.web_search_provider.search_label()
                } else {
                    command.label()
                };
                let mut control = Control::new(label, command.action());
                if command == ArtistCommand::SearchWeb {
                    control.search_provider = Some(state.web_search_provider);
                }
                control
            })
            .collect();
        return Some(Document {
            title: info.artist_name.clone(),
            subtitle: String::new(),
            kind: Kind::Artist,
            artist_artwork_visible: state
                .browse_nav()
                .and_then(|nav| {
                    state
                        .artist_info_column_index()
                        .and_then(|index| nav.columns.get(index))
                })
                .is_none_or(|column| column.artwork_visible),
            related_artists: Some(
                info.document
                    .related_artists
                    .iter()
                    .map(|artist| {
                        state
                            .library
                            .artists
                            .iter()
                            .chain(&state.library.track_artists)
                            .find(|known| known.rating_key == artist.rating_key)
                            .cloned()
                            .unwrap_or_else(|| artist.clone())
                    })
                    .collect(),
            ),
            text: artist_text(info).into(),
            loading: info.task.is_some(),
            controls,
            ..Default::default()
        });
    }

    let target = if state.view == View::Queue {
        state.queue.tracks.get(state.list_state.queue_index)
    } else {
        crate::app::sonic_details::selected_track(state).or_else(|| state.pane_track())
    };
    if let Some(track) = target {
        let following = if state.view == View::Queue {
            Some(&state.queue.tracks[state.list_state.queue_index..])
        } else if let Some(child) = crate::app::sonic_details::visible(state) {
            Some(&child.tracks[child.index..])
        } else {
            state
                .browse_nav()
                .and_then(|n| n.focused())
                .map(|c| &c.tracks[c.selected_index.min(c.tracks.len())..])
        };
        let controls = track_controls(track, following);
        return Some(Document {
            title: track.title.clone(),
            subtitle: track.track_artist().into(),
            rows: track_rows(track),
            track: Some(track.clone()),
            genres: track.genres.clone(),
            controls,
            ..Default::default()
        });
    }
    if let Some(column) = state.browse_nav().and_then(|n| n.focused())
        && let Some(lines) = column.detail_lines()
    {
        return Some(Document {
            kind: Kind::Album,
            title: column.title.clone(),
            track: column
                .detail_art_key()
                .and_then(|key| album_navigation_track(state, key)),
            genres: column
                .detail_art_key()
                .and_then(|key| album_navigation_track(state, key))
                .map(|track| track.genres)
                .unwrap_or_default(),
            rows: lines
                .iter()
                .map(|line| {
                    line.split_once(':')
                        .map(|(k, v)| (k.into(), v.trim().into()))
                        .unwrap_or_else(|| ("".into(), line.clone()))
                })
                .collect(),
            controls: column
                .play_all_row
                .as_ref()
                .map(|row| collection_controls(row, &column.ready_tracks()))
                .unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Some(nav) = state
        .folder_state
        .as_ref()
        .filter(|_| state.browse_category == BrowseCategory::Folders)
        && let Some(item) = nav.selected_item().filter(|i| i.is_track())
    {
        let track = nav
            .focused()
            .and_then(|column| column.tracks.get(&item.key))
            .cloned()
            .or_else(|| {
                state
                    .sources
                    .active
                    .folder()
                    .map(|id| Track::from_folder(id, &item.key))
            })?;
        return Some(Document {
            title: item.title.clone(),
            rows: track_rows(&track),
            subtitle: track.track_artist().into(),
            genres: track.genres.clone(),
            controls: vec![
                Control::new(
                    "Play Track",
                    FolderAction::PlayFolderTrack {
                        track_index: nav.focused().map_or(0, |c| c.selected_index),
                    },
                )
                .with_following("Play Track and Following", FolderAction::PlayFolderTracks),
                Control::new(
                    "Play Next",
                    QueueAction::EnqueueTracksNext(vec![track.clone()]),
                )
                .with_following(
                    "Play Track and Following Next",
                    QueueAction::EnqueueSelectionNext,
                ),
                Control::new(
                    "Add to Queue",
                    QueueAction::EnqueueTrack(Box::new(track.clone())),
                )
                .with_following(
                    "Add Track and Following to Queue",
                    QueueAction::EnqueueSelection,
                ),
            ],
            track: Some(track),
            ..Default::default()
        });
    }
    None
}

/// Context commands for the artist details currently selected in the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtistCommand {
    PreviousPhoto,
    NextPhoto,
    OpenSource,
    SearchWeb,
}
impl ArtistCommand {
    /// Canonical menu, palette and button title.
    pub fn label(self) -> &'static str {
        match self {
            Self::PreviousPhoto => "Previous Photo",
            Self::NextPhoto => "Next Photo",
            Self::OpenSource => "Open Source",
            Self::SearchWeb => "Search Web",
        }
    }
    /// Shared effect; the source reducer retains request ownership.
    pub fn action(self) -> SearchAction {
        match self {
            Self::PreviousPhoto => SearchAction::ArtistInfoPhoto(-1),
            Self::NextPhoto => SearchAction::ArtistInfoPhoto(1),
            Self::OpenSource => SearchAction::OpenArtistInfoSource,
            Self::SearchWeb => SearchAction::SearchArtistInfoOnWeb,
        }
    }
}
/// Commands supported by the loaded artist document.
pub fn artist_commands(info: &ArtistInfoDetails) -> Vec<ArtistCommand> {
    let mut commands = Vec::new();
    if info.document.images.len() > 1 {
        commands.extend([ArtistCommand::PreviousPhoto, ArtistCommand::NextPhoto]);
    }
    if info.document.source_url.is_some() {
        commands.push(ArtistCommand::OpenSource);
    }
    commands.push(ArtistCommand::SearchWeb);
    commands
}
/// Resolve the focused artist document without selecting an unrelated open pane.
pub fn focused_artist(state: &AppState) -> Option<&ArtistInfoDetails> {
    let index = state.artist_info_column_index()?;
    match state.browse_nav()?.columns.get(index)?.details.as_ref()? {
        BrowseDetails::Artist(info) => Some(info),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn artist_refresh_retains_cached_biography() {
        let work = tokio::spawn(std::future::pending::<()>());
        let info = ArtistInfoDetails {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
            request_id: 1,
            document: crate::services::biography::Biography {
                text: "Cached biography".into(),
                ..Default::default()
            },
            image_index: 0,
            task: Some(crate::app::tasks::TaskLease::new(&work)),
        };
        assert_eq!(artist_text(&info), "Cached biography");
        assert!(!related_message(None, false).unwrap().contains("Loading"));
        assert!(related_message(None, true).unwrap().contains("Loading"));
        assert!(
            related_message(Some(&Ok(vec![])), true)
                .unwrap()
                .contains("Loading")
        );
        work.abort();
    }
    #[test]
    fn artist_commands_require_their_document_capabilities() {
        let mut info = ArtistInfoDetails {
            artist_key: "artist".into(),
            artist_name: "Artist".into(),
            request_id: 1,
            document: Default::default(),
            image_index: 0,
            task: None,
        };
        assert_eq!(artist_commands(&info), vec![ArtistCommand::SearchWeb]);
        info.document.source_url = Some("https://example.org/artist".into());
        assert_eq!(
            artist_commands(&info),
            vec![ArtistCommand::OpenSource, ArtistCommand::SearchWeb]
        );
    }
}

/// Canonical metadata order for the Now Playing workspace.
pub fn now_playing_rows(track: &Track) -> Vec<(String, String)> {
    let mut rows = track_rows(track);
    if let Some(artist) = track.distinct_album_artist() {
        rows.push(("Album Artist".into(), artist.into()));
    }
    rows.sort_by_key(|(label, _)| match label.as_str() {
        "Artist" => 0,
        "Album Artist" => 1,
        "Album" => 2,
        "Genres" => 3,
        _ => 4,
    });
    rows
}

#[cfg(test)]
mod now_playing_tests {
    use super::*;

    #[test]
    fn album_artist_is_shown_only_when_the_recording_has_a_distinct_credit() {
        let mut track = Track {
            original_title: Some("Guest".into()),
            grandparent_title: Some("Ensemble".into()),
            ..Default::default()
        };
        let rows = now_playing_rows(&track);
        assert_eq!(rows[0], ("Artist".into(), "Guest".into()));
        assert_eq!(rows[1], ("Album Artist".into(), "Ensemble".into()));
        for artist in [Some(" GUEST "), Some(""), None] {
            track.grandparent_title = artist.map(String::from);
            assert!(
                !now_playing_rows(&track)
                    .iter()
                    .any(|(label, _)| label == "Album Artist")
            );
        }
    }
}
