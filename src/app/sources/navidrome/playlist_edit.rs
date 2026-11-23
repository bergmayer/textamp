//! Server playlist editing. An entry is identified by its position so repeated songs remain distinct.
use super::{commands::Command, *};
use crate::app::state::{
    BrowseColumn, BrowseItem, ColumnSortMode, PaletteCommandKind, PaletteEntry, PlayAllRow,
};
use crate::library::models::Playlist;

pub const PICKER: &str = "Add to Playlist";
#[derive(Clone, Debug)]
pub enum Addition {
    Track(String),
    Tracks(std::sync::Arc<[String]>),
    Album(String),
}
impl Addition {
    pub(super) async fn song_ids(self, session: &Session) -> anyhow::Result<Vec<String>> {
        Ok(match self {
            Addition::Track(key) => vec![session.id(&key)?],
            Addition::Tracks(keys) => keys
                .iter()
                .map(|k| session.id(k))
                .collect::<anyhow::Result<Vec<_>>>()?,
            Addition::Album(key) => {
                let tracks = session.fetch_album_tracks(&key).await?;
                anyhow::ensure!(!tracks.is_empty(), "Album contains no tracks");
                tracks
                    .iter()
                    .map(|t| session.id(&t.rating_key))
                    .collect::<anyhow::Result<Vec<_>>>()?
            }
        })
    }
    pub(super) fn belongs_to(&self, session: &Session) -> bool {
        match self {
            Self::Track(k) | Self::Album(k) => session.id(k).is_ok(),
            Self::Tracks(keys) => {
                !keys.is_empty() && keys.iter().all(|key| session.id(key).is_ok())
            }
        }
    }
}
impl From<Vec<Track>> for Addition {
    fn from(tracks: Vec<Track>) -> Self {
        Self::Tracks(tracks.into_iter().map(|t| t.rating_key).collect())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Up,
    Down,
    Remove,
}
impl Edit {
    pub const ALL: [Self; 3] = [Self::Up, Self::Down, Self::Remove];
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "Move Up in Playlist",
            Self::Down => "Move Down in Playlist",
            Self::Remove => "Remove from Playlist",
        }
    }
    pub fn available_for(
        self,
        row: usize,
        selected: &std::collections::BTreeSet<usize>,
        count: usize,
    ) -> bool {
        if selected.contains(&row) {
            return selected
                .first()
                .zip(selected.last())
                .is_some_and(|(&first, &last)| {
                    self.available(first, count) && self.available(last, count)
                });
        }
        self.available(row, count)
    }
    pub fn available(self, index: usize, count: usize) -> bool {
        index < count
            && match self {
                Self::Up => index > 0,
                Self::Down => index + 1 < count,
                Self::Remove => true,
            }
    }
}
fn owner<'a>(state: &'a AppState, col: &'a BrowseColumn) -> Option<&'a BrowseColumn> {
    if col.playlist_positions.is_empty() {
        return Some(col);
    }
    if col.sort_mode != ColumnSortMode::Default || !col.sort_ascending {
        return None;
    }
    let index = state
        .playlist_nav
        .columns
        .iter()
        .position(|c| std::ptr::eq(c, col))?;
    state
        .playlist_nav
        .columns
        .get(index.checked_sub(1)?)
        .filter(|c| c.grouped_by_album)
}

pub fn editable<'a>(state: &'a AppState, col: &'a BrowseColumn) -> Option<&'a str> {
    playlist_id(state, owner(state, col)?)
}

fn playlist_id<'a>(state: &AppState, col: &'a BrowseColumn) -> Option<&'a str> {
    let PlayAllRow::Playlist { rating_key, .. } = col.play_all_row.as_ref()? else {
        return None;
    };
    (state.sources.active.navidrome().is_some()
        && !state.downloads.is_offline()
        && col.sort_mode == ColumnSortMode::Default
        && col.sort_ascending
        && state
            .library
            .playlists
            .iter()
            .any(|p| p.rating_key == *rating_key && !p.smart))
    .then_some(rating_key)
}
pub fn set_editing(state: &mut AppState, playlist: &str, editing: bool) {
    let index = state.browse_nav().and_then(|nav| {
        nav.columns
            .iter()
            .position(|col| editable(state, col) == Some(playlist))
    });
    if let Some((nav, i)) = index.and_then(|i| state.browse_nav_mut().map(|nav| (nav, i))) {
        let col = &mut nav.columns[i];
        if col.playlist_editing != editing {
            col.playlist_editing = editing;
            col.grouped_playlist_marks.clear();
            col.selected_set.clear();
            col.selection_anchor = None;
        }
        if col.grouped_by_album {
            project_group_marks(&mut nav.columns, i);
        }
        nav.focused_column = i;
        state.category_column_focused = false;
    }
}

/// Rebuild row checkmarks from playlist occurrences after either column changes.
pub fn project_group_marks(columns: &mut [BrowseColumn], parent: usize) {
    let (parents, children) = columns.split_at_mut(parent + 1);
    let col = &mut parents[parent];
    let Some(groups) = &col.album_groups else {
        return;
    };
    col.selected_set = groups
        .iter()
        .enumerate()
        .filter_map(|(row, positions)| {
            positions
                .iter()
                .all(|p| col.grouped_playlist_marks.contains(p))
                .then_some(row)
        })
        .collect();
    if let Some(child) = children
        .first_mut()
        .filter(|c| !c.playlist_positions.is_empty())
    {
        child.playlist_editing = col.playlist_editing;
        child.selected_set = child
            .playlist_positions
            .iter()
            .enumerate()
            .filter_map(|(row, p)| col.grouped_playlist_marks.contains(p).then_some(row))
            .collect();
    }
}

/// Toggle all occurrences represented by an album, or one occurrence inside it.
pub fn mark_group_row(
    columns: &mut [BrowseColumn],
    index: usize,
    row: usize,
    checked: Option<bool>,
) -> bool {
    mark_group_rows(columns, index, [(row, checked)])
}

pub fn mark_group_rows(
    columns: &mut [BrowseColumn],
    index: usize,
    changes: impl IntoIterator<Item = (usize, Option<bool>)>,
) -> bool {
    let Some(column) = columns.get(index).filter(|c| {
        c.playlist_editing && c.sort_mode == ColumnSortMode::Default && c.sort_ascending
    }) else {
        return false;
    };
    let parent = if column.grouped_by_album {
        index
    } else if !column.playlist_positions.is_empty() && index > 0 {
        index - 1
    } else {
        return false;
    };
    let (parents, children) = columns.split_at_mut(parent + 1);
    let playlist = &mut parents[parent];
    let Some(groups) = playlist
        .album_groups
        .as_ref()
        .filter(|_| playlist.playlist_editing)
    else {
        return false;
    };
    let marks = &mut playlist.grouped_playlist_marks;
    let mut last = None;
    for (row, checked) in changes {
        let positions = if parent == index {
            groups.get(row).map(Vec::as_slice)
        } else {
            children[0]
                .playlist_positions
                .get(row)
                .map(std::slice::from_ref)
        };
        let Some(positions) = positions else {
            continue;
        };
        let checked = checked.unwrap_or_else(|| !positions.iter().all(|p| marks.contains(p)));
        for &position in positions {
            if checked {
                marks.insert(position);
            } else {
                marks.remove(&position);
            }
        }
        last = Some(row);
    }
    if let Some(row) = last {
        columns[index].header_selection = crate::app::state::ColumnHeaderSelection::Items;
        columns[index].selected_index = row;
        project_group_marks(columns, parent);
    }
    true
}

pub fn controls(
    state: &AppState,
    col: &BrowseColumn,
) -> Vec<crate::app::playback_controls::Control> {
    use crate::app::{action::BrowseAction, playback_controls::Control};
    let Some((parent, playlist)) =
        owner(state, col).and_then(|parent| playlist_id(state, parent).map(|id| (parent, id)))
    else {
        return vec![];
    };
    let mut controls = vec![Control::new(
        if col.playlist_editing { "Done" } else { "Edit" },
        BrowseAction::SetPlaylistEditing {
            playlist: playlist.into(),
            editing: !col.playlist_editing,
        },
    )];
    let marks = if parent.grouped_by_album {
        &parent.grouped_playlist_marks
    } else {
        &col.selected_set
    };
    if col.playlist_editing
        && let Some(&i) = marks.first()
    {
        for edit in Edit::ALL {
            if let Some(command) = occurrence_command(playlist, parent, marks, i, edit) {
                controls.push(Control::new(edit.label(), NavAction::Command(command)));
            }
        }
    }
    controls
}

/// Freeze selected occurrences when the chooser opens, including repeated song IDs.
pub fn addition(
    tracks: &[Track],
    selected: &std::collections::BTreeSet<usize>,
    row: usize,
) -> Option<Addition> {
    let track = tracks.get(row)?;
    if selected.len() > 1 && selected.contains(&row) {
        let keys = selected
            .iter()
            .map(|&i| tracks.get(i).map(|t| t.rating_key.clone()))
            .collect::<Option<Vec<_>>>()?;
        Some(Addition::Tracks(keys.into()))
    } else {
        Some(Addition::Track(track.rating_key.clone()))
    }
}

pub fn palette_addition(state: &AppState) -> Option<Addition> {
    let (tracks, row) = state.palette_track_list()?;
    let selected = if (state.palette.player_context
        || matches!(state.view, View::Queue | View::NowPlaying))
        && state.playback_mode != crate::app::state::PlaybackMode::Radio
    {
        &state.queue.selected
    } else {
        let col = state.browse_nav()?.focused()?;
        if !std::ptr::eq(tracks, col.tracks.as_slice()) {
            return None;
        }
        &col.selected_set
    };
    let first = *selected.first()?;
    addition(
        tracks,
        selected,
        if selected.contains(&row) { row } else { first },
    )
}

pub fn row_command(
    state: &AppState,
    col: &BrowseColumn,
    index: usize,
    edit: Edit,
) -> Option<Command> {
    let parent = owner(state, col)?;
    let playlist = playlist_id(state, parent)?;
    if parent.grouped_by_album {
        if !parent.playlist_editing {
            return None;
        }
        let positions = if std::ptr::eq(parent, col) {
            parent.album_groups.as_ref()?.get(index)?.as_slice()
        } else {
            std::slice::from_ref(col.playlist_positions.get(index)?)
        };
        let row = *positions.first()?;
        if positions
            .iter()
            .all(|p| parent.grouped_playlist_marks.contains(p))
        {
            return occurrence_command(playlist, parent, &parent.grouped_playlist_marks, row, edit);
        }
        return None;
    }
    occurrence_command(playlist, col, &col.selected_set, index, edit)
}

fn occurrence_command(
    playlist: &str,
    col: &BrowseColumn,
    marks: &std::collections::BTreeSet<usize>,
    index: usize,
    edit: Edit,
) -> Option<Command> {
    if col.playlist_editing && marks.is_empty() {
        return None;
    }
    if !edit.available_for(index, marks, col.tracks.len()) {
        return None;
    }
    let indices = if marks.contains(&index) {
        marks.iter().copied().collect()
    } else {
        vec![index]
    };
    let entries = indices
        .into_iter()
        .map(|i| col.tracks.get(i).map(|track| (i, track.rating_key.clone())))
        .collect::<Option<Vec<_>>>()?;
    Some(Command::EditEntry {
        playlist: playlist.into(),
        index,
        entries,
        edit,
    })
}
pub fn choose(state: &mut AppState, session: &Session, addition: Addition) {
    if !addition.belongs_to(session) || state.downloads.is_offline() {
        state.set_error("This selection requires its online Navidrome library".into());
        return;
    }
    state.palette.playlist_addition = Some((state.library_generation, addition));
    crate::app::command_palette::run(
        PaletteCommandKind::OpenCommandGroup(vec![PICKER.into()]),
        state,
    );
}
pub fn create_choice(command: &PaletteCommandKind) -> bool {
    matches!(
        command,
        PaletteCommandKind::Navidrome(Command::PromptNewPlaylist(Some(_)))
    )
}
pub fn choices(state: &AppState) -> Option<Vec<PaletteEntry>> {
    if !state.palette.open || state.palette.path != [PICKER] {
        return None;
    }
    let Some((_, addition)) = state
        .palette
        .playlist_addition
        .as_ref()
        .filter(|(generation, _)| *generation == state.library_generation)
    else {
        return Some(vec![]);
    };
    let mut entries = vec![PaletteEntry {
        label: "Create New Playlist…".into(),
        hint: String::new(),
        aliases: vec![],
        command: PaletteCommandKind::Navidrome(Command::PromptNewPlaylist(Some(addition.clone()))),
    }];
    entries.extend(
        state
            .library
            .playlists
            .iter()
            .filter(|p| !p.smart)
            .map(|p| PaletteEntry {
                label: p.title.clone(),
                hint: String::new(),
                aliases: vec![],
                command: PaletteCommandKind::Navidrome(Command::Append {
                    playlist: p.rating_key.clone(),
                    addition: addition.clone(),
                }),
            }),
    );
    let last_destination = state
        .active_library
        .as_ref()
        .and_then(|library| state.last_playlist_destinations.get(library));
    if let Some(last) = last_destination {
        let index = entries.iter().position(|entry| {
            matches!(&entry.command, PaletteCommandKind::Navidrome(Command::Append { playlist, .. }) if playlist == last)
        });
        if let Some(index) = index {
            entries[..=index].rotate_right(1);
        }
    }
    Some(entries)
}
pub enum Change {
    Append(Addition),
    Edit {
        expected: Vec<String>,
        indices: Vec<usize>,
        index: usize,
        edit: Edit,
    },
}
pub async fn save(
    session: Session,
    playlist: String,
    change: Change,
) -> anyhow::Result<Vec<Action>> {
    let id = session.id(&playlist)?;
    let store = session.cache_store()?;
    let gate = playlists::gate(&store, &id);
    let _guard = gate.lock().await;
    let before = read_complete(&session, &id).await?;
    let mut order: Vec<_> = before.entry.into_iter().map(|song| song.id).collect();
    let mut selection = None;
    let mut conflict = false;
    match change {
        Change::Append(addition) => {
            let added = addition.song_ids(&session).await?;
            let mut params = vec![("playlistId", id.clone())];
            params.extend(added.iter().cloned().map(|song| ("songIdToAdd", song)));
            session.client.call("updatePlaylist", &params).await?;
            order.extend(added);
        }
        Change::Edit {
            expected,
            mut indices,
            index,
            edit,
        } => {
            if order.iter().map(|song| session.key(song)).ne(expected) {
                conflict = true;
            } else {
                anyhow::ensure!(
                    indices.contains(&index)
                        && indices.windows(2).all(|w| w[0] < w[1])
                        && indices.iter().all(|&i| edit.available(i, order.len())),
                    "Playlist position is no longer valid"
                );
                match edit {
                    Edit::Remove => {
                        let mut params = vec![("playlistId", id.clone())];
                        params.extend(
                            indices
                                .iter()
                                .rev()
                                .map(|i| ("songIndexToRemove", i.to_string())),
                        );
                        session.client.call("updatePlaylist", &params).await?;
                        let first = indices[0];
                        let mut removed = indices.into_iter().peekable();
                        let mut position = 0;
                        order.retain(|_| {
                            let keep = removed.next_if_eq(&position).is_none();
                            position += 1;
                            keep
                        });
                        selection = Some((first, vec![]));
                    }
                    Edit::Up | Edit::Down => {
                        let dest = if edit == Edit::Up {
                            index - 1
                        } else {
                            index + 1
                        };
                        if edit == Edit::Up {
                            for i in &mut indices {
                                order.swap(*i, *i - 1);
                                *i -= 1;
                            }
                        } else {
                            for i in indices.iter_mut().rev() {
                                order.swap(*i, *i + 1);
                                *i += 1;
                            }
                        }
                        let mut params = vec![("playlistId", id.clone())];
                        params.extend(order.iter().cloned().map(|song| ("songId", song)));
                        session.client.call("createPlaylist", &params).await?;
                        selection = Some((dest, indices));
                    }
                }
            }
        }
    }
    let mut remote = read_complete(&session, &id).await.map_err(|e| {
        anyhow::anyhow!("Could not verify playlist contents. Refresh before retrying: {e}")
    })?;
    conflict |= remote.entry.iter().map(|song| &song.id).ne(&order);
    let tracks = std::mem::take(&mut remote.entry)
        .into_iter()
        .map(|song| session.track(song))
        .collect::<Vec<_>>();
    let playlist = session.playlist(remote);
    if let Err(error) =
        playlists::save_edited(store.ticket(&format!("playlist:{id}")), &playlist, &tracks).await
    {
        tracing::warn!("Cache edited playlist: {error:#}");
    }
    Ok(vec![
        NavAction::PlaylistEdited {
            playlist,
            tracks,
            selection,
            conflict,
        }
        .into(),
        crate::app::action::DataAction::LoadPlaylists.into(),
    ])
}

async fn read_complete(
    session: &Session,
    id: &str,
) -> anyhow::Result<crate::navidrome::models::Playlist> {
    let playlist = session.client.playlist(id).await?;
    anyhow::ensure!(
        playlist.id == id
            && playlist
                .song_count
                .is_none_or(|count| count as usize == playlist.entry.len()),
        "Server returned an incomplete or different playlist"
    );
    Ok(playlist)
}

pub fn applied(
    state: &mut AppState,
    playlist: Playlist,
    tracks: Vec<Track>,
    selection: Option<(usize, Vec<usize>)>,
    conflict: bool,
) -> Vec<Action> {
    let mut followups = Vec::new();
    let key = &playlist.rating_key;
    let mut lists = state.library.playlists.to_vec();
    if let Some(p) = lists.iter_mut().find(|p| p.rating_key == *key) {
        *p = playlist.clone();
    }
    state.replace_playlists(lists);
    let nav = &state.playlist_nav;
    let root_matches = nav
        .columns
        .first()
        .and_then(|c| c.selected_item())
        .is_some_and(|item| item.key() == key);
    let current_column = nav.columns.iter().position(|col| match &col.play_all_row {
        Some(PlayAllRow::Playlist { rating_key, .. }) => rating_key == key,
        _ => false,
    });
    if let Some(ci) = current_column.filter(|_| root_matches) {
        state.sources.nav_tasks.remove("playlist-browse");
        state.playlist_nav_request_id = state.playlist_nav_request_id.wrapping_add(1);
        let open_album =
            state
                .playlist_nav
                .columns
                .get(ci + 1)
                .and_then(|child| match &child.play_all_row {
                    Some(PlayAllRow::Album { rating_key, .. })
                        if !child.playlist_positions.is_empty() =>
                    {
                        Some((
                            rating_key.clone(),
                            state.playlist_nav.focused_column == ci + 1,
                        ))
                    }
                    _ => None,
                });
        state.playlist_nav.columns.truncate(ci + 1);
        state.playlist_nav.focused_column = state.playlist_nav.focused_column.min(ci);
        let old = &mut state.playlist_nav.columns[ci];
        let count = tracks.len();
        let mut col = BrowseColumn::new_with_tracks(
            format!(
                "{} — {count} {}",
                playlist.title,
                if count == 1 { "track" } else { "tracks" }
            ),
            BrowseItem::from_tracks(&tracks),
            tracks,
        );
        col.play_all_row = old.play_all_row.clone();
        col.header_selection = old.header_selection;
        col.artwork_visible = old.artwork_visible;
        col.lazy = old.lazy.clone();
        if let Some(l) = &mut col.lazy {
            l.total = Some(count as u32);
            l.loading = false;
        }
        if old.grouped_by_album {
            col.group_by_album();
        } else if old.sort_mode != ColumnSortMode::Default {
            col.apply_sort(old.sort_mode);
        }
        if col.sort_ascending != old.sort_ascending {
            col.items.reverse();
            col.tracks.reverse();
            col.sort_ascending = old.sort_ascending;
        }
        col.playlist_editing = old.playlist_editing;
        let mut cursor = old.selected_index;
        if let Some((index, indices)) = selection
            .filter(|_| !conflict && col.sort_mode == ColumnSortMode::Default && col.sort_ascending)
        {
            if col.grouped_by_album {
                col.grouped_playlist_marks = indices.into_iter().collect();
                cursor = col
                    .album_groups
                    .as_ref()
                    .and_then(|groups| groups.iter().position(|g| g.contains(&index)))
                    .unwrap_or(cursor);
                project_group_marks(std::slice::from_mut(&mut col), 0);
            } else {
                cursor = index;
                if old.playlist_editing || indices.len() > 1 {
                    col.selected_set = indices.into_iter().collect();
                    col.selection_anchor = Some(index);
                }
            }
        }
        col.selected_index = cursor.min(col.items.len().saturating_sub(1));
        *old = col;
        if let Some((key, focused)) = open_album
            && let Some(row) = old.items.iter().position(|item| item.key() == key)
            && let Some(child) = crate::app::handlers::helpers::drill_grouped_album(old, row)
        {
            old.selected_index = row;
            state.playlist_nav.columns.push(child);
            if focused {
                state.playlist_nav.focused_column = ci + 1;
            }
        }
        if state.list_filter.active
            && state.list_filter.category == BrowseCategory::Playlists
            && state.list_filter.column >= ci
        {
            if state.list_filter.column == ci && state.browse_category == BrowseCategory::Playlists
            {
                state.list_filter.results = None;
                followups.push(
                    crate::app::action::SearchAction::SetListFilterQuery(
                        state.list_filter.query.clone(),
                    )
                    .into(),
                );
            } else {
                state.list_filter.deactivate();
            }
        }
    }
    if conflict {
        state.set_error(
            "Playlist changed on the server. The list has been refreshed. Select the entry again."
                .into(),
        );
    } else {
        state.set_status(format!("Updated playlist “{}”", playlist.title));
    }
    followups
}

#[cfg(test)]
mod grouped_tests {
    use super::*;
    #[test]
    fn last_playlist_destination_is_first_only_for_its_library() {
        let mut state = AppState::new();
        state.palette.open = true;
        state.palette.path = vec![PICKER.into()];
        state.palette.playlist_addition =
            Some((state.library_generation, Addition::Track("song".into())));
        state.library.playlists = ["One", "Two"]
            .into_iter()
            .map(|key| {
                serde_json::from_value::<Playlist>(
                    serde_json::json!({"ratingKey":key,"key":key,"title":key,"type":"audio"}),
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
            .into();
        state.active_library = Some("library".into());
        state
            .last_playlist_destinations
            .insert("library".into(), "Two".into());
        let labels = |state: &AppState| {
            choices(state)
                .unwrap()
                .into_iter()
                .map(|entry| entry.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(&state), ["Two", "Create New Playlist…", "One"]);
        state.active_library = Some("other".into());
        assert_eq!(labels(&state), ["Create New Playlist…", "One", "Two"]);
    }

    #[test]
    fn grouped_playlist_edits_target_original_occurrences() {
        let tracks = [("r", "A"), ("other", "B"), ("r", "A"), ("last", "B")]
            .into_iter()
            .map(|(key, album)| Track {
                rating_key: key.into(),
                parent_rating_key: Some(album.into()),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut parent =
            BrowseColumn::new_with_tracks("Playlist", BrowseItem::from_tracks(&tracks), tracks);
        parent.play_all_row = Some(PlayAllRow::Playlist {
            rating_key: "list".into(),
            title: "List".into(),
        });
        parent.group_by_album();
        parent.playlist_editing = true;
        let child = crate::app::handlers::helpers::drill_grouped_album(&parent, 0).unwrap();
        let mut columns = vec![parent, child];
        assert!(mark_group_row(&mut columns, 0, 0, None));
        assert_eq!(
            columns[1].selected_set.iter().copied().collect::<Vec<_>>(),
            [0, 1]
        );
        assert!(mark_group_row(&mut columns, 1, 0, None));
        assert!(columns[0].selected_set.is_empty());
        let parent = &columns[0];
        let command = occurrence_command(
            "list",
            parent,
            &parent.grouped_playlist_marks,
            2,
            Edit::Remove,
        );
        assert!(
            matches!(command,Some(Command::EditEntry{entries,..}) if entries == [(2,"r".into())])
        );
        assert!(mark_group_row(&mut columns, 0, 1, None));
        assert_eq!(
            columns[0]
                .grouped_playlist_marks
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        let child = crate::app::handlers::helpers::drill_grouped_album(&columns[0], 0).unwrap();
        assert_eq!(child.selected_set.iter().copied().collect::<Vec<_>>(), [1]);
    }
}
