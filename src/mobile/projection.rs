use super::commands::{Adapter, Binding};
use crate::app::{self, AppState, action::*, state::*};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

const ALPHABET_INDEX_THRESHOLD: usize = 270;

/// Bounded immutable image handoff. The persistent source cache remains authoritative.
#[derive(Default)]
pub(super) struct Assets {
    pub(super) bytes: Arc<HashMap<String, Arc<Vec<u8>>>>,
    order: std::collections::VecDeque<String>,
    total_bytes: usize,
    artwork_revision: Option<(u64, u64)>,
    artwork_ids: HashMap<String, String>,
}
impl Assets {
    fn insert(&mut self, bytes: &[u8]) -> String {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hash);
        let id = format!("{:x}", hash.finish());
        if !self.bytes.contains_key(&id) {
            while (self.order.len() >= 512
                || self.total_bytes.saturating_add(bytes.len()) > 64 * 1024 * 1024)
                && let Some(old) = self.order.pop_front()
            {
                if let Some(bytes) = Arc::make_mut(&mut self.bytes).remove(&old) {
                    self.total_bytes = self.total_bytes.saturating_sub(bytes.len());
                }
            }
            self.order.push_back(id.clone());
            Arc::make_mut(&mut self.bytes).insert(id.clone(), Arc::new(bytes.to_vec()));
            self.total_bytes += bytes.len();
        }
        id
    }
    fn artwork(&mut self, state: &AppState, key: &str, kind: &str) -> String {
        let revision = (state.library_generation, state.artwork.grid_revision);
        if self.artwork_revision != Some(revision) {
            self.artwork_revision = Some(revision);
            self.artwork_ids.clear();
        }
        if let Some(bytes) = state.artwork.grid_cache.get(key) {
            if let Some(id) = self.artwork_ids.get(key)
                && self.bytes.contains_key(id)
            {
                return id.clone();
            }
            let id = self.insert(bytes);
            self.artwork_ids.insert(key.into(), id.clone());
            return id;
        }
        self.placeholder(kind)
    }
    fn placeholder(&mut self, kind: &str) -> String {
        let id = format!("placeholder-{kind}");
        if self.bytes.contains_key(&id) {
            return id;
        }
        Arc::make_mut(&mut self.bytes)
            .entry(id.clone())
            .or_insert_with(|| {
                Arc::new(match kind {
                    "artist" => include_bytes!("../../assets/artwork/artist.png").to_vec(),
                    "album" => include_bytes!("../../assets/artwork/album.png").to_vec(),
                    _ => include_bytes!("../../assets/artwork/track.png").to_vec(),
                })
            });
        id
    }
}
fn button(
    state: &AppState,
    adapter: &mut Adapter,
    identity: &str,
    label: &str,
    binding: Binding,
) -> Value {
    let disabled = match &binding {
        Binding::Palette(command) | Binding::PaletteSelection(command) => {
            app::command_catalog::disabled_reason(state, command)
        }
        Binding::Actions(actions)
            if state.downloads.is_offline()
                && actions.iter().any(|action| {
                    matches!(
                        action,
                        Action::System(SystemAction::Downloads(
                            app::downloads::Command::Tracks(_)
                                | app::downloads::Command::Album(_)
                                | app::downloads::Command::Playlist(_)
                        ))
                    )
                }) =>
        {
            Some("Unavailable offline")
        }
        Binding::TrackContext {
            kind: app::track_context::ContextKind::Download,
            ..
        } if state.downloads.is_offline() => Some("Unavailable offline"),
        _ => None,
    };
    use app::track_context::ContextKind as K;
    let group = match &binding {
        Binding::PlaylistEntry { .. }
        | Binding::ToggleTrackSelection { .. }
        | Binding::TrackContext {
            kind: K::AddToPlaylist,
            ..
        } => "Playlist",
        Binding::TrackContext {
            kind: K::SonicRadio | K::SonicAdventure,
            ..
        } => "Discovery",
        Binding::TrackContext {
            kind: K::ShowDetails(_) | K::ArtistBio { .. } | K::OpenInLibrary,
            ..
        } => "Details",
        Binding::TrackContext {
            kind: K::Download, ..
        } => "Downloads",
        Binding::Actions(actions) => match actions.first() {
            Some(Action::System(SystemAction::Downloads(_))) => "Downloads",
            Some(Action::Browse(BrowseAction::OpenDetailsInLibrary { .. })) => "Details",
            Some(Action::Source(app::sources::SourceAction::Navidrome(nav)))
                if matches!(
                    nav.as_ref(),
                    app::sources::navidrome::NavAction::Command(
                        app::sources::navidrome::commands::Command::ChoosePlaylist(_)
                    )
                ) =>
            {
                "Playlist"
            }
            _ => "Playback",
        },
        _ => "Playback",
    };
    json!({"menuGroup":group,"controlId":identity,"token":adapter.bind(state,identity,binding),"label":label,"disabled":disabled})
}
fn palette_identity(command: &PaletteCommandKind, label: &str) -> String {
    let identity = match command {
        PaletteCommandKind::OpenCommandGroup(path) => {
            return format!("palette-group:{}", path.join("/"));
        }
        PaletteCommandKind::FromTrackContext { kind, track } => {
            format!("{kind:?}:{}", track.rating_key)
        }
        PaletteCommandKind::JumpToArtist(track) => {
            format!(
                "artist:{}",
                track.grandparent_rating_key.as_deref().unwrap_or("")
            )
        }
        PaletteCommandKind::JumpToAlbum(track) => {
            format!("album:{}", track.parent_rating_key.as_deref().unwrap_or(""))
        }
        _ => format!("{command:?}"),
    };
    format!("palette-command:{label}:{identity}")
}
fn actions(
    state: &AppState,
    adapter: &mut Adapter,
    owner: &str,
    controls: Vec<app::playback_controls::Control>,
) -> Vec<Value> {
    let mut primary = vec![];
    for control in controls {
        let mut primary_button = button(
            state,
            adapter,
            &format!("{owner}:{}", control.label),
            &control.label,
            Binding::Actions(control.actions.as_ref().clone()),
        );
        primary_button["mark"] = json!(control.search_provider.map(|provider| provider.mark()));
        primary_button["symbol"] = json!(control.actions.iter().find_map(|action| match action {
            Action::Queue(
                QueueAction::PlayTrack(_)
                | QueueAction::PlayAlbumNow { .. }
                | QueueAction::PlayPlaylistNow { .. }
                | QueueAction::PlayTracksNow(_),
            )
            | Action::Folders(
                FolderAction::PlayFolderTrack { .. } | FolderAction::PlayFolderTracks,
            )
            | Action::Radio(RadioAction::PlayStation(_)) => Some("play.fill"),
            Action::Radio(RadioAction::StartArtistRadio { .. }) =>
                Some("dot.radiowaves.left.and.right"),
            Action::Source(app::sources::SourceAction::Navidrome(nav)) => match nav.as_ref() {
                app::sources::navidrome::NavAction::Command(
                    app::sources::navidrome::commands::Command::EditEntry { edit, .. },
                ) => Some(match edit {
                    app::sources::navidrome::playlist_edit::Edit::Up => "arrow.up",
                    app::sources::navidrome::playlist_edit::Edit::Down => "arrow.down",
                    app::sources::navidrome::playlist_edit::Edit::Remove => "trash",
                }),
                _ => None,
            },
            Action::Search(SearchAction::OpenArtistInfoSource) => Some("safari"),
            Action::Search(SearchAction::ArtistInfoPhoto(delta)) => Some(if *delta < 0 {
                "chevron.left"
            } else {
                "chevron.right"
            }),
            Action::Queue(
                QueueAction::EnqueueAlbumNext { .. } | QueueAction::EnqueueTracksNext(_),
            ) => Some("text.line.first.and.arrowtriangle.forward"),
            Action::Queue(
                QueueAction::EnqueueAlbum { .. }
                | QueueAction::TracksLoaded {
                    intent: QueueLoadIntent::Append { .. },
                    ..
                },
            ) => Some("text.badge.plus"),
            Action::System(SystemAction::Downloads(
                app::downloads::Command::Tracks(_)
                | app::downloads::Command::Album(_)
                | app::downloads::Command::Playlist(_),
            )) => Some("arrow.down.circle"),
            Action::System(SystemAction::Downloads(app::downloads::Command::Remove(_))) =>
                Some("trash"),
            _ => None,
        }));
        if let Some((label, actions)) = control.alternate {
            primary_button["context"] = json!([button(
                state,
                adapter,
                &format!("{owner}:{label}"),
                &label,
                Binding::Actions(actions.as_ref().clone()),
            )]);
        }
        primary.push(primary_button);
    }
    primary
}
fn jump(
    track: crate::library::models::Track,
    destination: app::library_details::Destination,
) -> Binding {
    Binding::Actions(vec![
        BrowseAction::OpenDetailsInLibrary {
            track: Box::new(track),
            destination,
        }
        .into(),
    ])
}
fn details(state: &AppState, adapter: &mut Adapter, assets: &mut Assets) -> Option<Value> {
    let doc = app::details::project(state)?;
    let key = state
        .browse_nav()
        .and_then(|nav| nav.focused())
        .and_then(BrowseColumn::detail_art_key)
        .or_else(|| {
            doc.track
                .as_ref()
                .map(|t| t.parent_rating_key.as_deref().unwrap_or(&t.rating_key))
        })
        .unwrap_or("")
        .to_owned();
    Some(detail_json(state, adapter, assets, doc, &key))
}

fn related_artist_buttons(
    state: &AppState,
    adapter: &mut Adapter,
    assets: &mut Assets,
    artists: &[crate::library::models::Artist],
) -> Vec<Value> {
    artists
        .iter()
        .map(|artist| {
            let track = crate::library::models::Track {
                grandparent_rating_key: Some(artist.rating_key.clone()),
                grandparent_title: Some(artist.title.clone()),
                ..Default::default()
            };
            let mut item = button(
                state,
                adapter,
                &format!("related:{}", artist.rating_key),
                &artist.title,
                jump(track, app::library_details::Destination::Artist),
            );
            item["artwork"] = json!(assets.artwork(state, &artist.rating_key, "artist"));
            item
        })
        .collect()
}

fn detail_json(
    state: &AppState,
    adapter: &mut Adapter,
    assets: &mut Assets,
    doc: app::details::Document,
    key: &str,
) -> Value {
    let kind = match doc.kind {
        app::details::Kind::Artist => "artist",
        app::details::Kind::Album => "album",
        app::details::Kind::Track => "track",
    };
    let identity = if kind == "album" {
        format!("details:album:{key}")
    } else {
        format!(
            "details:{kind}:{}",
            doc.track
                .as_ref()
                .map(|track| track.rating_key.as_str())
                .unwrap_or(key)
        )
    };
    let bio = doc.artist_bio.map(|control| {
        let mut command = button(
            state,
            adapter,
            &format!("{identity}:artist-bio"),
            &control.label,
            Binding::Actions(control.actions.as_ref().clone()),
        );
        command["mark"] = json!(control.search_provider.map(|provider| provider.name()));
        command
    });
    let mut links = vec![];
    let mut album_artist = None;
    if let Some(track) = doc.track.as_ref() {
        if doc.kind == app::details::Kind::Album {
            if !doc.subtitle.is_empty() && state.artist_destination_for_track(track).is_some() {
                album_artist = Some(button(
                    state,
                    adapter,
                    &format!("{identity}:album-artist"),
                    &doc.subtitle,
                    jump(track.clone(), app::library_details::Destination::Artist),
                ));
            }
        } else {
            for (label, destination) in [
                (
                    track.artist_name(),
                    app::library_details::Destination::Artist,
                ),
                (track.album_name(), app::library_details::Destination::Album),
            ] {
                if !label.is_empty() {
                    links.push(button(
                        state,
                        adapter,
                        &format!("{identity}:link:{label}"),
                        label,
                        jump(track.clone(), destination),
                    ));
                }
            }
        }
    }
    let related = related_artist_buttons(
        state,
        adapter,
        assets,
        doc.related_artists.as_deref().unwrap_or_default(),
    );
    let similar = doc
        .similar
        .as_ref()
        .map(|similar| {
            similar
                .tracks
                .iter()
                .map(|track| {
                    let mut item = button(
                        state,
                        adapter,
                        &format!("{identity}:similar:{}", track.rating_key),
                        &format!(
                            "{} — {}",
                            if similar.albums {
                                track
                                    .grandparent_title
                                    .as_deref()
                                    .unwrap_or_else(|| track.artist_name())
                            } else {
                                track.artist_name()
                            },
                            if similar.albums {
                                track.album_name()
                            } else {
                                &track.title
                            }
                        ),
                        jump(
                            track.clone(),
                            if similar.albums {
                                app::library_details::Destination::Album
                            } else {
                                app::library_details::Destination::Track
                            },
                        ),
                    );
                    if similar.albums
                        && let Some(key) = &track.parent_rating_key
                    {
                        item["artwork"] = json!(assets.artwork(state, key, "album"));
                        item["subtitle"] = json!(track.album_name());
                        item["description"] = json!(track.artist_name());
                    }
                    item
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({"id":identity,"title":doc.title,"kind":kind,"subtitle":doc.subtitle,"rows":doc.rows,"text":doc.text,
        "review":doc.album_info.as_ref().map(|info| &info.review),"albumURL":doc.album_info.as_ref().and_then(|info|info.display_link().map(|url|url.as_str())),
        "artistBio":bio,"lyrics":doc.lyrics,"loading":doc.loading,"artwork":if kind == "track" { doc.track.as_ref().filter(|_|doc.track_artwork).map(|t|assets.artwork(state,&t.rating_key,kind)) } else {Some(assets.artwork(state,key,kind))},"actions":actions(state,adapter,&identity,doc.controls),
        "albumArtist":album_artist,"links":links,"related":related,"similar":similar,"relatedTitle":doc.kind.related_label(),
        "message":doc.similar.map(|value|value.message).unwrap_or_default()})
}

fn view_options(state: &AppState, adapter: &mut Adapter, popup: &SortPopupState) -> Vec<Value> {
    let owner = super::commands::owner(state, popup.column_idx);
    app::view_options::choices(state, popup).into_iter().map(|choice| {
        json!({"menuGroup":if matches!(choice.id, SortPopupOption::SortMode(_) | SortPopupOption::Direction) {"Sort"} else {"Display"},"token":adapter.bind(state,&format!("{owner}:option:{:?}",choice.id),Binding::ViewOption{column:popup.column_idx,option:choice.id}),"label":choice.label,"checked":choice.checked})
    }).collect()
}

fn artist_web_search(state: &AppState, adapter: &mut Adapter, owner: &str, name: &str) -> Value {
    let provider = state.web_search_provider;
    let mut action = button(
        state,
        adapter,
        owner,
        provider.search_label(),
        Binding::Actions(vec![
            SystemAction::OpenBrowser(provider.artist_url(name)).into(),
        ]),
    );
    action["mark"] = json!(provider.name());
    action
}

fn dialog(state: &AppState, adapter: &mut Adapter) -> Option<Value> {
    let mut choices = vec![];
    let mut actions = vec![];
    let mut search = None;
    let mut query = None;
    let mut selected = None;
    let (title, text, loading) = if let Some(popup) = state.popups.sort() {
        choices = view_options(state, adapter, popup);
        ("View Options".into(), popup.column_title.clone(), false)
    } else if let Some(picker) = state.popups.artist_radio_picker() {
        query = Some(picker.query.clone());
        let artist_button =
            |adapter: &mut Adapter, artist: &crate::library::models::Artist, select| {
                button(
                    state,
                    adapter,
                    &format!("artist-radio:{select}:{}", artist.rating_key),
                    &artist.title,
                    Binding::ArtistRadio {
                        key: artist.rating_key.clone(),
                        select,
                    },
                )
            };
        selected = Some(
            picker
                .selected_artists
                .iter()
                .map(|artist| artist_button(adapter, artist, false))
                .collect::<Vec<_>>(),
        );
        if !picker.query.trim().is_empty() {
            choices = picker
                .filtered_artists
                .iter()
                .filter(|artist| {
                    !picker
                        .selected_artists
                        .iter()
                        .any(|chosen| chosen.rating_key == artist.rating_key)
                })
                .map(|artist| {
                    let mut choice = artist_button(adapter, artist, true);
                    if picker.selected_artists.len()
                        >= app::state::ArtistRadioPickerState::MAX_SELECTED
                    {
                        choice["disabled"] = json!("Choose up to 12 artists");
                    }
                    choice
                })
                .collect();
        }
        let mut play = button(
            state,
            adapter,
            "artist-radio:play",
            "Play Mix",
            Binding::Actions(vec![SearchAction::ArtistRadioPickerLaunch.into()]),
        );
        play["disabled"] = json!(
            picker
                .selected_artists
                .is_empty()
                .then_some("Select artists")
        );
        actions.push(play);
        ("Artist Mix".into(), String::new(), false)
    } else if let Some(bio) = state.popups.artist_bio() {
        search = Some(artist_web_search(
            state,
            adapter,
            "bio-search",
            &bio.artist_name,
        ));
        if bio.document.source_url.is_some() {
            actions.push(button(
                state,
                adapter,
                "bio-source",
                "Open Source",
                Binding::Actions(vec![app::action::SearchAction::OpenBiographySource.into()]),
            ));
        }
        (
            bio.artist_name.clone(),
            bio.document.text.clone(),
            bio.task.is_some(),
        )
    } else if let Some(popup) = state.popups.text() {
        (popup.title.clone(), popup.text.clone(), false)
    } else {
        return None;
    };
    Some(
        json!({"selected":selected,"search":search,"title":title,"text":text,"loading":loading,"choices":choices,"actions":actions,"query":query}),
    )
}

/// Navidrome installs playlists and radio destinations with its first browse
/// catalog. Later track loading and refreshes can run with navigation visible.
pub(super) fn library_loading(state: &AppState) -> bool {
    use app::sources::ActiveSource;
    match &state.sources.active {
        ActiveSource::None => state.library_loading || state.sources.nav_connection_task.is_some(),
        ActiveSource::Navidrome(_) => {
            state.library.artists_loading
                && matches!(
                    state.library.catalog_loading,
                    app::catalog_loading::State::Browsing { .. }
                )
        }
        ActiveSource::Folder(_) => false,
    }
}

pub(super) fn model(state: &AppState, adapter: &mut Adapter, assets: &mut Assets) -> Value {
    adapter.begin_projection();
    let opening = library_loading(state);
    let library_stations = state
        .station_nav
        .columns
        .first()
        .filter(|_| !opening)
        .into_iter()
        .flat_map(|root| root.stations())
        .map(|station| {
            let command = crate::app::radio_navigation::shortcut_command(station);
            let mut item = button(
                state,
                adapter,
                &format!("library-radio:{}", station.key),
                &station.title,
                Binding::Palette(command),
            );
            item["value"] = json!(station.identifier.as_deref().unwrap_or(&station.key));
            item
        })
        .collect::<Vec<_>>();
    let sidebar = if opening {
        Vec::new()
    } else {
        app::sidebar::sidebar(state)
    };
    let mut groups = Vec::new();
    for group in sidebar.iter().filter_map(|row| row.parent) {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    let sidebar_groups = groups
        .into_iter()
        .map(|group| {
            let mut command = button(
                state,
                adapter,
                &format!("sidebar-group:{}", group.label()),
                group.label(),
                Binding::SidebarGroup(group),
            );
            command["canExpand"] = app::sidebar::allows_disclosure(&sidebar, group.label()).into();
            command
        })
        .collect::<Vec<_>>();
    let roots=sidebar.into_iter().filter(|row|!row.header).map(|row| {
        let label=row.label.clone(); let section=row.parent.map(|parent|parent.label()).unwrap_or("Library");
        let context = playlist_context(state, adapter, row.playlist_key.as_deref());
        let active = state.library_navigation.has_columns() && row.active;
        let identity = row.playlist_key.as_deref().unwrap_or(&label);
        json!({"active":active,"token":adapter.bind(state,&format!("sidebar:{section}:{identity}"),Binding::Sidebar(row)),"label":label,"section":section,"context":context})
    }).collect::<Vec<_>>();
    let radio = state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Radio;
    let mut columns=state.browse_nav().filter(|_| state.library_navigation.has_columns() && !radio).map(|nav|nav.columns.iter().enumerate().skip(super::commands::hidden_root_columns(state)).map(|(index,column)| {
        let filtering=state.list_filter.active && state.list_filter.category==state.browse_category && state.list_filter.column==index;
        let matched = state.list_filter.results.as_ref().filter(|_| filtering).map(|r| r.matched_indices.as_slice());
        let count = matched.map_or(column.items.len(), <[usize]>::len);
        let row_index = |position: usize| matched.map_or(position, |rows| rows[position]);
        let owner = super::commands::owner(state,index);
        let selected = matched.map_or(column.selected_index, |rows| rows.iter().position(|&i| i == column.selected_index).unwrap_or(0));
        let range = adapter.window(&owner,count,selected);
        let album_key = match &column.play_all_row { Some(PlayAllRow::Album { rating_key, .. }) => Some(rating_key.as_str()), _ => None };
        let more = column.lazy.as_ref().is_some_and(|lazy|lazy.total.is_none_or(|total|column.tracks.len() < total as usize));
        let sections = if count > ALPHABET_INDEX_THRESHOLD { alphabet_sections((0..count).map(|p| &column.items[row_index(p)])) } else { vec![] };
        let grid = app::view_options::album_grid(state,index);
        let grid_start = grid.then(||(0..count).position(|p| matches!(column.items.get(row_index(p)),Some(BrowseItem::Album{..})))).flatten();
        let rows=range.clone().enumerate().filter_map(|(offset,p)| {
            let i = row_index(p);
            let item=column.items.get(i)?;
            let (kind,subtitle,year)=match item {
                BrowseItem::Album{artist,year,..}=>("album",artist.clone(),*year),
                BrowseItem::Artist{..}=>("artist",String::new(),None),
                BrowseItem::Track{artist_name,duration_ms,..}=>("track",format!("{} · {}",artist_name.as_deref().unwrap_or(""),crate::util::format_duration(*duration_ms)),None),
                _=>("folder",String::new(),None)
            };
            let pending = item.pending_download().and_then(|id|app::downloads::pending_status(state,id));
            let download = app::downloads::item_command(state,column,i).map(|(label,command)|button(state,adapter,&format!("{owner}:download-item:{}",item.key()),label,Binding::Actions(vec![command.into()])));
            let cancel = if pending.is_some() {download.clone()} else {None};
            let mut context = if cancel.is_some() {vec![]} else {row_context(state,adapter,&owner,column,i,item)};
            if let Some(download) = download {context.retain(|entry|entry["menuGroup"] != "Downloads");context.insert(0,download);}
            let balanced_caption = if kind == "album" {
                let caption = app::view_options::album_caption(item.title(), year);
                app::view_options::caption_lines(&caption).map(|(a,b)| format!("{a}\n{b}"))
            } else { None };
            Some(json!({"balancedCaption":balanced_caption,"pending":pending,"cancel":cancel,"id":format!("{i}:{}",item.key()),"key":item.key(),"index":i,"position":range.start+offset,"title":item.title(),"subtitle":subtitle,"kind":kind,"year":year,"context":context,
                "artwork": if (grid || column.artwork_visible) && (kind=="album" || kind=="artist") {Some(assets.artwork(state,item.key(),kind))} else if column.artwork_visible && kind == "track" && album_key.is_none() {
                    column.tracks.get(i).and_then(|track| track.parent_rating_key.as_deref()).map(|key| assets.artwork(state,key,"album"))
                } else {None},"batchSelected":column.selected_set.contains(&i),"selected":if column.selected_set.is_empty(){i==column.selected_index}else{column.selected_set.contains(&i)}}))
        }).collect::<Vec<_>>();
        let options=app::view_options::for_column(state,index).map(|popup|view_options(state,adapter,&popup)).unwrap_or_default();
        let edit_controls = app::sources::navidrome::playlist_edit::controls(state,column);
        let editable = !edit_controls.is_empty();
        let editing = editable && column.playlist_editing;
        let edit_buttons = actions(state,adapter,&owner,edit_controls);
        let mut playback = if !column.tracks.is_empty() { column.play_all_row.as_ref().map(|row|actions(state,adapter,&format!("{owner}:playback"),app::playback_controls::collection_controls(row,&column.ready_tracks()))).unwrap_or_default() } else { vec![] };
        if index == 1 && app::sources::audiomuse::description_results(state).is_some() {
            let mut save = button(state, adapter, &format!("{owner}:save-playlist"), "Save as Playlist", Binding::Actions(vec![app::sources::audiomuse::Command::PromptSavePlaylist.into()]));
            save["symbol"] = json!("square.and.arrow.down");
            playback.push(save);
        }
        let parent_item = index.checked_sub(1).and_then(|parent| nav.columns[parent].selected_item());
        let (title, named_destination) = match &column.play_all_row {
            Some(PlayAllRow::Album { title, .. } | PlayAllRow::Playlist { title, .. }) => (title.as_str(), true),
            _ => match parent_item {
                Some(item @ BrowseItem::Artist { .. }) => (item.title(), true),
                _ => (column.title.as_str(), matches!(parent_item, Some(BrowseItem::Genre { .. } | BrowseItem::Description { .. } | BrowseItem::Album { .. } | BrowseItem::Playlist { .. }))),
            },
        };
        let title = column.artist_header.as_ref().map_or(title, |info|info.artist_name.as_str());
        let album_page = album_key.and_then(|key| app::details::album_page(state, column)
            .map(|doc| detail_json(state, adapter, assets, doc, key)));
        let artist_header = column.artist_header.as_ref().map(|info| {
            let action = SearchAction::ShowArtistBio { artist_key: info.artist_key.clone(), artist_name: info.artist_name.clone() };
            let reader = button(state,adapter,&format!("{owner}:artist-bio"),"Artist Biography",Binding::Actions(vec![action.into()]));
            let mix = app::column_controls::artist_mix(state, column).map(|control| {
                button(state, adapter, &format!("{owner}:artist-mix"), &control.label, Binding::Actions(control.actions.as_ref().clone()))
            });
            json!({"mix":mix,"artwork":assets.artwork(state,&info.artist_key,"artist"),"text":info.excerpt(),"reader":reader,"loading":info.task.is_some(),"search":info.needs_web_search().then(||artist_web_search(state,adapter,&format!("{owner}:artist-search"),&info.artist_name))})
        });
        json!({"id":owner,"gridEnd":grid.then(||(0..count).rfind(|p|matches!(column.items.get(row_index(*p)),Some(BrowseItem::Album{..}))).map(|p|p+1)).flatten(),"gridStart":grid_start,"artistRelated":column.artist_header.as_ref().map(|info| related_artist_buttons(state,adapter,assets,&info.document.related_artists)), "artistHeader":artist_header,"albumPage":album_page,"namedDestination":named_destination,"playlistEditControls":edit_buttons,"playlistEditing":editing,"title":title,"compactTitle":if title == column.title && column.play_all_row.is_none() {crate::util::column_heading::destination(title)} else {title},"rows":rows,"more":more,"count":count,"summary":if state.desktop_library_root == LibraryRoot::Downloads && index == 0 {Some(app::downloads::summary(state,count))}else{None},"sections":sections,"selectedPosition":selected,"detail":column.details.is_some(),
            "filter":if filtering {state.list_filter.query.as_str()}else{""},"filterActive":filtering,"options":options,"playback":playback,
             "albumDetailsArtwork":album_key.map(|key| assets.artwork(state,key,"album")),"albumArtworkKey":album_key,
            "descriptionPrompt":index == 0 && state.desktop_library_root == LibraryRoot::DescribeMusic})

    }).collect::<Vec<_>>()).unwrap_or_default();
    if state.library_navigation.has_columns() && state.browse_category == BrowseCategory::Folders {
        columns = state.folder_state.as_ref().map(|nav| nav.columns.iter().enumerate().map(|(index, column)| {
            let filtering = state.list_filter.active && state.list_filter.category == BrowseCategory::Folders && state.list_filter.column == index;
            let matched = state.list_filter.results.as_ref().filter(|_| filtering).map(|r| r.matched_indices.as_slice());
            let count = matched.map_or(column.items.len(), <[usize]>::len);
            let row_index = |position: usize| matched.map_or(position, |rows| rows[position]);
            let owner = super::commands::owner(state,index);
            let selected = matched.map_or(column.selected_index, |rows| rows.iter().position(|&i| i == column.selected_index).unwrap_or(0));
            let range = adapter.window(&owner,count,selected);
            let sections = if count > ALPHABET_INDEX_THRESHOLD { title_sections((0..count).map(|position| (position,column.items[row_index(position)].title.as_str()))) } else { vec![] };
            let rows = range.clone().enumerate().map(|(offset,position)| {
                let index = row_index(position);
                let item = &column.items[index];
                json!({"id":item.key,"key":item.key,"index":index,"position":range.start+offset,"title":item.title,"subtitle":"","kind":if item.is_track(){"track"}else{"folder"},"year":null,"artwork":state.sources.active.folder().and_then(|source| {let dir=if item.is_track(){item.key.rsplit_once('/').map_or("",|(dir,_)|dir)}else{&item.key};let key=crate::library::track::Track::folder_cover_key(source,dir);state.artwork.grid_cache.contains_key(&key).then(||assets.artwork(state,&key,"album"))}),"batchSelected":column.selected_set.contains(&index),"selected":index==column.selected_index})
            }).collect::<Vec<_>>();
            json!({"id":owner,"namedDestination":index > 0,"title":column.title,"rows":rows,"count":count,"sections":sections,"selectedPosition":selected,"detail":false,"filter":if filtering{state.list_filter.query.as_str()}else{""},"filterActive":filtering,"options":[]})
        }).collect()).unwrap_or_default();
    }
    if state.library_navigation.has_columns()
        && state.browse_category == BrowseCategory::Library
        && state.desktop_library_root == LibraryRoot::Radio
    {
        columns=state.station_nav.columns.iter().map(|column| {
            let owner=super::commands::station_owner(state.library_generation,column);
            let rows=column.stations().iter().enumerate().map(|(index,station)| {
                let token=adapter.bind(state,&format!("{owner}:{}",station.key),Binding::Station{owner:owner.clone(),key:station.key.clone(),open:station.requires_input()});
                let disclosure=adapter.bind(state,&format!("{owner}:{}:open",station.key),Binding::Station{owner:owner.clone(),key:station.key.clone(),open:true});
                json!({"disclosure":disclosure,"id":station.key,"key":station.key,"index":index,"title":station.title,"subtitle":"","kind":"station","artwork":null,"selected":column.selected_index==index,"activation":token})
            }).collect::<Vec<_>>();
            json!({"id":owner,"namedDestination":column.key.is_some(),"title":if column.key.is_none(){"Radio"}else{&column.title},"stationDescription":column.key.as_deref().and_then(|key|state.station_by_key(key)).map(|station|station.description_text()),"count":rows.len(),"rows":rows,"detail":false,"radio":true,"filter":"","options":[]})
        }).collect();
        if let Some(key) = state.station_detail.as_deref()
            && let Some(station) = state.station_by_key(key)
        {
            let identity = format!("radio-detail:{}:{}", state.library_generation, key);
            let play = button(
                state,
                adapter,
                &format!("{identity}:play"),
                "Play Station",
                Binding::Actions(vec![RadioAction::PlayStation(station.key.clone()).into()]),
            );
            columns.push(json!({"id":identity,"namedDestination":true,"title":station.title,"count":0,"rows":[],"detail":false,"radio":true,"filter":"","options":[],"stationDetails":{"title":station.title,"text":station.description_text(),"input":if station.requires_input() {if station.identifier.as_deref()==Some("artist-picker"){Some("artists")}else{Some("track")}}else{None},"actions":if station.requires_input(){vec![]}else{vec![play]}}}));
        }
    }
    if let Some(group) = state.library_navigation.group() {
        let owner = super::commands::group_owner(state, group);
        let query = adapter
            .group_filters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        let items = app::sidebar::filtered_group_rows(state, group.label(), &query);
        let matched = 0..items.len();
        let sections = if matched.len() > ALPHABET_INDEX_THRESHOLD {
            title_sections(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, (_, row))| (index, row.label.as_str())),
            )
        } else {
            vec![]
        };
        let rows = matched.into_iter().enumerate().map(|(index, original)| {
            let row = items[original].1.clone();
            let label = row.label.clone();
            let context = playlist_context(state, adapter, row.playlist_key.as_deref());
            let selected = state.library_navigation.has_columns() && row.active;
            let identity = row.playlist_key.as_deref().unwrap_or(&label).to_owned();
            let token = adapter.bind(state, &format!("sidebar:{}:{identity}", group.label()), Binding::Sidebar(row));
            json!({"id":format!("sidebar:{identity}"),"key":identity,"index":index,"title":label,"subtitle":"","kind":"category","selected":selected,"activation":token,"context":context})
        }).collect::<Vec<_>>();
        columns.insert(0, json!({"id":owner,"title":group.label(),"count":rows.len(),"rows":rows,"sections":sections,"detail":false,"sidebar":true,"filter":query,"filterActive":!query.is_empty(),"options":[]}));
    }
    let owners = columns
        .iter()
        .filter_map(|column| column["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    adapter
        .windows
        .retain(|owner, _| owners.contains(owner.as_str()));
    adapter
        .group_filters
        .retain(|owner, _| owners.contains(owner.as_str()));
    let playing_index = match state.playback_mode {
        PlaybackMode::Radio => state.radio.track_index,
        _ => state.queue.index,
    };
    let mut occurrences = HashMap::<String, usize>::new();
    let queue=state.playback_tracks().iter().enumerate().map(|(index,track)| { let occurrence = occurrences.entry(track.rating_key.clone()).or_default(); let id = format!("{}:{}", track.rating_key, *occurrence); *occurrence += 1; json!({"id":id,"key":track.rating_key,"index":index,"title":track.title,"subtitle":format!("{} · {}",track.artist_name(),track.album_name()),"kind":"track","year":null,
        "artwork":assets.artwork(state,track.parent_rating_key.as_deref().unwrap_or(&track.rating_key),"album"),"selected":playing_index==Some(index),
        "details":button(state,adapter,&format!("queue:{id}:details"),"Track Details",jump(track.clone(),app::library_details::Destination::Track))})}).collect::<Vec<_>>();
    let queue_modes = app::queue_modes::choices(state)
        .into_iter()
        .map(|choice| {
            let mut control = button(
                state,
                adapter,
                &format!("queue-mode:{}", choice.label),
                &choice.label,
                Binding::Palette(choice.command),
            );
            control["group"] = json!(choice.group);
            control["active"] = json!(choice.selected);
            control["disabled"] = json!(choice.disabled);
            control
        })
        .collect::<Vec<_>>();
    let tools = app::queue_controls::definitions(state)
        .into_iter()
        .map(|tool| {
            let identity = format!("queue-tool:{:?}", tool.tool);
            json!({"controlId":identity,"token":adapter.bind(state,&identity,Binding::Tool(tool.tool)),"label":tool.label,"description":tool.description,"menuGroup":tool.group,"active":tool.active,"disabled":tool.disabled_reason})
        })
        .collect::<Vec<_>>();
    let settings=SettingsPage::ALL.into_iter().flat_map(|page|app::preferences::page(state,page).into_iter().filter_map(move |preference| {
        if matches!(preference.id,TextampSetting::Presentation(_)) {return None;}
        Some((page,preference))
    })).map(|(page,preference)| { let (kind, bounds) = match &preference.value { app::preferences::Value::Toggle{..} => ("toggle", None), app::preferences::Value::Choice{..} => ("choice", None), app::preferences::Value::Count{current,min,max,..} => ("count", Some([*current,*min,*max])), app::preferences::Value::Text{..} => ("text", None) }; json!({"kind":kind,"bounds":bounds,"token":adapter.bind(state,&format!("preference:{:?}",preference.id),Binding::Actions(vec![SettingsAction::ActivatePreference(preference.id).into()])),"label":preference.label,"section":if page == SettingsPage::Sidebar {"Library"} else {page.name()},"group":preference.group,"description":preference.description,"checked":preference.value.checked(),"value":preference.value.display(),"choices":if preference.id == TextampSetting::Transcode {
            Some(app::preferences::TRANSCODE_BITRATES.into_iter().map(|bitrate| {
                let label = if bitrate == 0 { "Original audio".into() } else { format!("Up to {bitrate} kbps") };
                let mut choice = button(state, adapter, &format!("transcode:{bitrate}"), &label,
                    Binding::Actions(vec![SettingsAction::SetTranscodeBitrate(bitrate).into()]));
                choice["checked"] = json!(state.transcode_kbps == bitrate);
                choice
            }).collect::<Vec<_>>())
        } else { None }}) }).collect::<Vec<_>>();
    let discovery = [
        (PaletteCommandKind::OpenRadioBrowser, "Radio", "r"),
        (PaletteCommandKind::LibraryMix, "Library Mix", "l"),
        (
            PaletteCommandKind::PlayRandomAlbums,
            "Play Random Albums",
            "a",
        ),
        (PaletteCommandKind::SonicAdventure, "Journey", "j"),
        (
            PaletteCommandKind::GoToRandomAlbum,
            "Go to Random Album",
            "R",
        ),
        (PaletteCommandKind::OpenAiBrowser, "AI", ""),
    ]
    .into_iter()
    .map(|(command, label, key)| {
        let mut item = button(
            state,
            adapter,
            &format!("discover:{label}"),
            label,
            Binding::Palette(command),
        );
        item["shortcut"] = json!(key);
        item
    })
    .collect::<Vec<_>>();
    let external_searches = crate::services::external_search::SearchTarget::ALL
        .into_iter()
        .filter(|target| app::external_search::enabled(state, *target))
        .map(|target| {
            let entry = app::external_search::entry(target);
            let mut control = button(
                state,
                adapter,
                &format!("external-search:{target:?}"),
                &entry.label,
                Binding::Palette(entry.command),
            );
            control["disabled"] = json!(app::external_search::disabled_reason(state, target));
            control
        })
        .collect::<Vec<_>>();
    let libraries = app::sources::library_choices(state).into_iter().filter_map(|choice| {
        let key = choice.artwork_key()?;
        let options = app::sources::options::Options { choice: choice.clone(), selected: 0, page: Default::default(), scroll_pin: None };
        let pages: Vec<_> = options.pages(state).into_iter().map(|page| {
            let mut options = options.clone(); options.page = page;
            let controls: Vec<_> = options.items(state).into_iter().filter(|(action, _)| *action != app::sources::options::OptionAction::Close).map(|(action, label)| {
                let mut control = button(state, adapter, &format!("library-option:{key}:{action:?}"), &label, Binding::LibraryOption { choice: choice.clone(), action });
                control["kind"] = json!(match action { app::sources::options::OptionAction::Connection => Some("connection"), app::sources::options::OptionAction::AudioMuse => Some("audiomuse"), _ => None });
                if action == app::sources::options::OptionAction::Sonic {
                    control["kind"] = json!("toggle");
                    control["label"] = json!("Sonic / AI features");
                    control["checked"] = json!(app::sources::sonic::choice_key(&choice, state)
                        .is_some_and(|key| !state.sources.sonic_disabled_libraries.contains(&key)));
                }
                control["destructive"] = json!(matches!(action, app::sources::options::OptionAction::Remove | app::sources::options::OptionAction::Clear | app::sources::options::OptionAction::ClearArtwork));
                control
            }).collect();
            json!({"title":page.name(),"actions":controls})
        }).collect();
        Some(json!({"id":key,"provider":choice.provider().to_lowercase(),"pages":pages,"token":adapter.bind(state,&format!("library:{key}"),Binding::Actions(vec![choice.action()])),"label":choice.label(),"subtitle":choice.provider(),"checked":choice.active(state),
            "refresh":button(state,adapter,&format!("refresh:{key}"),"Refresh Cache",Binding::Actions(vec![SettingsAction::RescanSourceCache(choice.clone()).into()]))}))
    }).collect::<Vec<_>>();
    let caches = app::sources::cache::summaries(state).into_iter().filter_map(|cache| {
        let key = cache.choice.artwork_key()?;
        let controls = cache.controls.into_iter().map(|(action,label)| {
            let mut control = button(state, adapter, &format!("cache:{key}:{action:?}"), label,
                Binding::LibraryOption { choice: cache.choice.clone(), action });
            control["destructive"] = json!(matches!(action, app::sources::options::OptionAction::Clear | app::sources::options::OptionAction::ClearArtwork));
            control
        }).collect::<Vec<_>>();
        Some(json!({"id":key,"label":cache.choice.label(),"metadata":cache.metadata,"artwork":cache.artwork,"status":cache.status,"running":cache.running,"actions":controls}))
    }).collect::<Vec<_>>();
    let notice = state
        .notifications
        .last_error
        .as_ref()
        .map(|message| {
            let recovery = match state.notifications.error_target {
                Some(app::state::NotificationTarget::RefreshLibrary) => Some(adapter.bind(
                    state,
                    "retry-library",
                    Binding::Actions(vec![SystemAction::RefreshCategory(RefreshCategory::Artists).into()]),
                )),
                _ => None,
            };
            json!({"message":message,"kind":"error","action":recovery,"actionLabel":recovery.map(|_| "Retry")})
        })
        .or_else(|| {
            state
                .notifications
                .toast_message
                .as_ref()
                .map(|message| json!({"message":message,"kind":"confirmation"}))
        })
        .or_else(|| {
            state
                .notifications
                .status_message
                .as_ref()
                .map(|message| json!({"message":message,"kind":"information","action":state.notifications.status_target.map(|_|adapter.bind(state,"download-notice",Binding::Actions(vec![app::downloads::Command::Open.into()])))}))
        });
    let downloads = app::downloads::Menu::MANAGEMENT
        .into_iter()
        .map(|command| {
            button(
                state,
                adapter,
                &format!("downloads:{}", command.label()),
                command.label(),
                Binding::Palette(PaletteCommandKind::Downloads(command)),
            )
        })
        .collect::<Vec<_>>();
    let search = search(state, adapter);
    let supported_groups = app::command_catalog::groups(
        state
            .palette
            .entries
            .iter()
            .filter(|entry| {
                !matches!(entry.command, PaletteCommandKind::OpenCommandGroup(_))
                    && palette_supported(&entry.command)
            })
            .map(|entry| app::command_catalog::path(&entry.command)),
    );
    let palette_back = state
        .palette
        .matches
        .iter()
        .filter_map(|index| state.palette.entries.get(*index))
        .find(|entry| {
            entry.label == "Back"
                && matches!(entry.command, PaletteCommandKind::OpenCommandGroup(_))
        })
        .map(|entry| {
            button(
                state,
                adapter,
                "palette-back",
                &entry.label,
                Binding::Palette(entry.command.clone()),
            )
        });
    let palette = state
        .palette
        .matches
        .iter()
        .filter_map(|index| state.palette.entries.get(*index))
        .filter(|entry| palette_supported(&entry.command))
        .filter(|entry| match &entry.command {
            PaletteCommandKind::OpenCommandGroup(path) => {
                entry.label != "Back" && supported_groups.contains(path)
            }
            _ => true,
        })
        .take(100)
        .map(|entry| {
            let mut control = button(
                state,
                adapter,
                &palette_identity(&entry.command, &entry.label),
                &entry.label,
                Binding::PaletteSelection(entry.command.clone()),
            );
            control["disabled"] =
                json!(app::command_catalog::disabled_reason(state, &entry.command));
            control["submenu"] = json!(matches!(
                entry.command,
                PaletteCommandKind::OpenCommandGroup(_)
            ));
            control
        })
        .collect::<Vec<_>>();
    let connection = state
        .popups
        .library_dialog()
        .and_then(|dialog| match dialog {
            app::sources::dialogs::Dialog::Navidrome(form) => {
                Some(json!({"owner":form.instance,"provider":"navidrome","editing":form.editing,"fields":form.fields.iter().map(|field|field.value.to_string()).collect::<Vec<_>>(),"busy":form.busy,"error":form.error}))
            }
            app::sources::dialogs::Dialog::Webdav(form) => {
                Some(json!({"owner":form.instance,"provider":"webdav","editing":form.editing,"fields":form.fields.iter().map(|field|field.value.to_string()).collect::<Vec<_>>(),"busy":form.task.is_some(),"error":form.error}))
            }
            app::sources::dialogs::Dialog::AudioMuse(form) => Some(json!({"owner":form.instance,"provider":"audiomuse","editing":true,"fields":form.fields.iter().map(|field|field.value.to_string()).collect::<Vec<_>>(),"busy":form.task.is_some(),"error":form.error})),
            _ => None,
        });
    let journey = state.popups.adventure_launcher().map(|launcher| {
        let choices = app::journey::choices(launcher).iter().enumerate().map(|(index,choice)|json!({
            "token":adapter.bind(state,&format!("journey-choice:{}",choice.key()),Binding::JourneyChoice{index,key:choice.key().into()}),"label":choice.title(),"subtitle":choice.subtitle(),"kind":match choice { app::journey::Choice::Artist(_) => "artist", app::journey::Choice::Album(_) => "album", app::journey::Choice::Track(_) => "track" }
        })).collect::<Vec<_>>();
        let mut controls = vec![("Back",SearchAction::AdventureLauncherBack)];
        match &launcher.stage {
            JourneyStage::Ready(_) => controls.extend([("Play Journey",SearchAction::AdventureLauncherPlay),("Edit Journey",SearchAction::AdventureLauncherEdit)]),
            JourneyStage::Generating { .. } => {},
            _ if launcher.purpose == TrackPickerPurpose::Journey => controls.extend([(app::journey::SWAP,SearchAction::AdventureLauncherReverse),(app::journey::PREVIEW,SearchAction::AdventureLauncherGenerate)]),
            _ => controls.push(("Play Station",SearchAction::StartPickedTrackRadio)),
        }
        let controls = controls.into_iter().map(|(label,action)|button(state,adapter,&format!("journey:{label}"),label,Binding::Actions(vec![action.into()]))).collect::<Vec<_>>();
        json!({"start":launcher.start_track.as_ref().map(|track|track.title.as_str()).unwrap_or(""),"end":launcher.end_track.as_ref().map(|track|track.title.as_str()).unwrap_or(""),"count":launcher.track_count_input,"query":launcher.query,
            "drilled":!matches!(launcher.drill,AdventureDrillLevel::Search),"ready":matches!(launcher.stage,JourneyStage::Ready(_)),"radio":launcher.purpose != TrackPickerPurpose::Journey,"loading":launcher.loading || matches!(launcher.stage,JourneyStage::Generating { .. }),"choices":choices,"actions":controls,"error":if let JourneyStage::Failed(error)=&launcher.stage {Some(error)}else{None}})
    });
    let file_asset = "playlist-document";
    let mut exporting = false;
    let input = state.popups.input_dialog().map(|input| {
        let (file_mode, generation) = match &input.action_type {
            InputDialogAction::ExportPlaylist {generation, contents, ..} => {
                exporting = true;
                if !assets.bytes.get(file_asset).is_some_and(|old| Arc::ptr_eq(old, contents)) {
                    Arc::make_mut(&mut assets.bytes).insert(file_asset.into(), contents.clone());
                }
                (Some("export"), Some(*generation))
            }
            InputDialogAction::ImportPlaylist {generation} => (Some("import"), Some(*generation)),
            _ => (None,None),
        };
        json!({"title":input.title,"text":input.input.to_string(),"submit":input.action_type.submit_label(),
            "fileMode":file_mode,"generation":generation,"asset":exporting.then_some(file_asset)})
    });
    if !exporting && assets.bytes.contains_key(file_asset) {
        Arc::make_mut(&mut assets.bytes).remove(file_asset);
    }
    let confirmation = state
        .popups
        .confirm_dialog()
        .map(|dialog| json!({"title":dialog.title,"text":dialog.message}));
    let mut model = json!({"libraryNavigationId":format!("{}:{:?}:{:?}",state.library_generation,state.desktop_library_root,state.jump_history.head()),"tab":adapter.tab,"sheet":adapter.sheet,"settingsOpen":state.view == View::Settings,"libraryLoading":opening,"roots":roots,"sidebarGroups":sidebar_groups,"columns":columns,"detail":details(state,adapter,assets),"trackDetail":state.track_pane_open,"canReturn":state.jump_history.head().is_some(),
        "queueMode":app::queue_modes::label(state),"queueModes":queue_modes,"externalSearches":external_searches,"queue":queue,"history":state.listening_history.iter().map(|p|json!({"id":p.id,"artist":p.artist,"album":p.album,"title":p.title,"time":p.time})).collect::<Vec<_>>(),"tools":tools,"settings":settings,"libraries":libraries,"caches":caches,"search":search,"searchQuery":state.search.query,"searchPending":app::handlers::dispatch_search::search_pending(state),"palette":palette,"palettePath":state.palette.path,"paletteBack":palette_back,"connection":connection,
        "theme":theme(state.theme),"artworkMode":state.artwork.mode.name(),"loading":app::activity::message(state),
        "notice":notice,"activityAction":app::activity::target(state).map(|_|adapter.bind(state,"download-activity",Binding::Actions(vec![app::downloads::Command::Open.into()]))),"offline":state.downloads.is_offline(),
        "lyricsView":app::lyrics::view(state),"lyrics":state.popups.text().map(|popup|popup.text.clone()),"downloads":downloads,"singlesLimit":state.singles_track_limit,"journey":journey,"input":input,"confirmation":confirmation,"dialog":dialog(state,adapter)});
    model["libraryStations"] = Value::Array(library_stations);
    model["discovery"] = Value::Array(discovery);
    if let Some(doc) = app::details::now_playing_details(state) {
        model["playingDetail"] = detail_json(state, adapter, assets, doc, "");
    }
    adapter.finish_projection();
    model
}

fn playlist_context(state: &AppState, adapter: &mut Adapter, key: Option<&str>) -> Vec<Value> {
    key.map(|key| {
        vec![button(
            state,
            adapter,
            &format!("playlist:{key}:download"),
            "Download",
            Binding::Actions(vec![app::downloads::Command::Playlist(key.into()).into()]),
        )]
    })
    .unwrap_or_default()
}

fn row_context(
    state: &AppState,
    adapter: &mut Adapter,
    owner: &str,
    column: &BrowseColumn,
    index: usize,
    item: &BrowseItem,
) -> Vec<Value> {
    let identity = format!("{owner}:context:{index}:{}", item.key());
    match item {
        BrowseItem::Description { query } => {
            let mut remove = button(
                state,
                adapter,
                &format!("{identity}:remove"),
                "Remove from History",
                Binding::Actions(vec![
                    app::sources::audiomuse::Command::RemoveDescription {
                        query: query.clone(),
                    }
                    .into(),
                ]),
            );
            remove["destructive"] = json!(true);
            remove["kind"] = json!("removeDescription");
            vec![remove]
        }
        BrowseItem::Album {
            key, title, artist, ..
        } => {
            let mut entries = actions(
                state,
                adapter,
                &identity,
                app::playback_controls::album_controls(key, title),
            );
            if state
                .sources
                .active
                .navidrome()
                .is_some_and(|s| s.id(key).is_ok())
                && !state.downloads.is_offline()
            {
                use app::sources::navidrome::{
                    NavAction, commands::Command, playlist_edit::Addition,
                };
                entries.push(button(
                    state,
                    adapter,
                    &format!("{identity}:playlist"),
                    "Add Album to Playlist…",
                    Binding::Actions(vec![
                        NavAction::Command(Command::ChoosePlaylist(Addition::Album(key.clone())))
                            .into(),
                    ]),
                ));
            }
            let track = crate::library::models::Track {
                parent_rating_key: Some(key.clone()),
                parent_title: Some(title.clone()),
                grandparent_title: Some(artist.clone()),
                ..Default::default()
            };
            entries.push(button(
                state,
                adapter,
                &format!("{identity}:details"),
                "Open in Library",
                jump(track, app::library_details::Destination::Album),
            ));
            entries.push(button(
                state,
                adapter,
                &format!("{identity}:download"),
                "Download",
                Binding::Actions(vec![app::downloads::Command::Album(key.clone()).into()]),
            ));
            entries
        }
        BrowseItem::Playlist { key, .. } => playlist_context(state, adapter, Some(key)),
        BrowseItem::Artist { key, title, .. } => {
            let track = crate::library::models::Track {
                grandparent_rating_key: Some(key.clone()),
                grandparent_title: Some(title.clone()),
                ..Default::default()
            };
            vec![button(
                state,
                adapter,
                &format!("{identity}:details"),
                "Open in Library",
                jump(track, app::library_details::Destination::Artist),
            )]
        }
        BrowseItem::Track { key, .. } => {
            let Some(track) = column
                .tracks
                .get(index)
                .filter(|track| track.rating_key == *key)
                .or_else(|| column.tracks.iter().find(|track| track.rating_key == *key))
            else {
                return vec![];
            };
            let policy = app::track_context::ContextPolicy {
                queued: false,
                ..app::track_context::ContextPolicy::for_state(state)
            };
            let mut entries: Vec<_> =
                app::track_context::track_context_with_policy(policy, track, false)
                    .into_iter()
                    .filter(|entry| {
                        !matches!(
                            entry.kind,
                            app::track_context::ContextKind::Separator
                                | app::track_context::ContextKind::SearchExternal(_)
                        )
                    })
                    .map(|entry| {
                        button(
                            state,
                            adapter,
                            &format!("{identity}:{}", entry.label),
                            if matches!(entry.kind, app::track_context::ContextKind::AddToPlaylist)
                                && column.selected_set.len() > 1
                                && column.selected_set.contains(&index)
                            {
                                "Add Selected Tracks to Playlist…"
                            } else {
                                &entry.label
                            },
                            Binding::TrackContext {
                                owner: owner.into(),
                                index,
                                key: key.clone(),
                                kind: entry.kind,
                            },
                        )
                    })
                    .collect();
            if policy.server_playlists && key.starts_with("navidrome:") {
                let marked = column.selected_set.contains(&index);
                entries.push(button(
                    state,
                    adapter,
                    &format!("{identity}:selection"),
                    if marked {
                        "Deselect Track"
                    } else {
                        "Select Track"
                    },
                    Binding::ToggleTrackSelection {
                        owner: owner.into(),
                        index,
                        key: key.clone(),
                    },
                ));
            }
            if app::sources::navidrome::playlist_edit::editable(state, column).is_some() {
                for edit in app::sources::navidrome::playlist_edit::Edit::ALL {
                    if edit.available_for(index, &column.selected_set, column.tracks.len()) {
                        entries.push(button(
                            state,
                            adapter,
                            &format!("{identity}:{edit:?}"),
                            edit.label(),
                            Binding::PlaylistEntry {
                                owner: owner.into(),
                                index,
                                key: key.clone(),
                                edit,
                            },
                        ));
                    }
                }
            }
            entries
        }
        _ => vec![],
    }
}

fn alphabet_sections<'a>(items: impl Iterator<Item = &'a BrowseItem>) -> Vec<Value> {
    title_sections(items.enumerate().filter_map(|(position, item)| {
        (!matches!(
            item,
            BrowseItem::AllArtists
                | BrowseItem::AllTracks { .. }
                | BrowseItem::ArtistInfo { .. }
                | BrowseItem::Compilations
                | BrowseItem::CompilationTracks { .. }
        ))
        .then_some((position, item.title()))
    }))
}

fn title_sections<'a>(titles: impl Iterator<Item = (usize, &'a str)>) -> Vec<Value> {
    use app::handlers::helpers::{ALPHABET_STRIP_LETTERS, sort_letter};
    let mut sections = vec![];
    let mut seen = std::collections::HashSet::new();
    for (position, title) in titles {
        let first = sort_letter(title, 0).unwrap_or('%');
        let letter = match first {
            c if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
            c if c.is_ascii_digit() => '0',
            c if c.is_ascii() => '%',
            _ => '文',
        };
        if !ALPHABET_STRIP_LETTERS.contains(&letter.to_ascii_lowercase()) || !seen.insert(letter) {
            continue;
        }
        if sections.is_empty() && position > 0 {
            sections.push(json!({"label":"","position":0}));
        }
        sections.push(json!({"label":letter.to_string(),"position":position}));
    }
    sections
}

fn search(state: &AppState, adapter: &mut Adapter) -> Vec<Value> {
    if state.search.query.is_empty() {
        return state.search.recent.iter().enumerate().map(|(index,result)| {
            let id = format!("recent-result:{index}:{}", result.key);
            json!({"token":adapter.bind(state,&id,Binding::RecentSearch(result.clone())),"label":result.label,"subtitle":result.kind_label(),"section":"Recently Chosen"})
        }).collect();
    }
    let Some(results) = &state.search.results else {
        return vec![];
    };
    let query = &state.search.query;
    (0..results.total_count()).map(|row| {
        let (tab, index) = app::handlers::dispatch_search::resolve_global_index(results, row);
        let (title, subtitle) = match tab {
            SearchTab::Artists => (results.artists[index].title.as_str(), ""),
            SearchTab::Albums => (results.albums[index].title.as_str(), results.albums[index].artist_name()),
            SearchTab::Playlists => (results.playlists[index].title.as_str(), ""),
            SearchTab::Genres => (results.genres[index].title.as_str(), ""),
            SearchTab::Tracks => (results.tracks[index].title.as_str(), results.tracks[index].track_artist()),
            SearchTab::Global => unreachable!(),
        };
        let kind = tab.result_kind();
        let subtitle = if subtitle.is_empty() { kind.into() } else { format!("{kind} · {subtitle}") };
        json!({"token":adapter.bind(state,&format!("search:{query}:{}:{index}",tab.name()),Binding::Search(tab,index,query.clone())),"label":title,"subtitle":subtitle,"section":tab.name()})
    }).collect()
}
fn palette_supported(command: &PaletteCommandKind) -> bool {
    !matches!(
        command,
        PaletteCommandKind::CycleApplicationMode
            | PaletteCommandKind::SetApplicationMode(_)
            | PaletteCommandKind::ToggleFullscreen
            | PaletteCommandKind::FullscreenArtwork
            | PaletteCommandKind::DesktopFullscreen(_)
            | PaletteCommandKind::Quit
            | PaletteCommandKind::OpenHelp
    )
}
fn color(color: app::theme_palette::Color) -> Option<String> {
    let [r, g, b] = color.rgb()?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}
fn theme(name: app::theme::ThemeName) -> Value {
    use app::theme_palette::{Color, Palette};
    let palette = Palette::<Color>::for_theme(name);
    let p = palette.map(color);
    json!({"name":name.config_name(),"background":p.bg_primary,"surface":p.bg_secondary,"foreground":p.fg_primary,"secondary":p.fg_secondary,"accent":p.fg_accent,"border":p.border,"errorForeground":p.error})
}
pub(super) fn player(state: &AppState, assets: &mut Assets) -> Value {
    let track = state.current_track();
    let artwork = match state.current_artwork_bytes() {
        Some(bytes) => assets.insert(bytes),
        None => assets.placeholder("track"),
    };
    json!({"key":track.map(|t|t.rating_key.as_str()),"title":track.map(|t|t.title.as_str()).unwrap_or("Nothing playing"),"artist":track.map(|t|t.track_artist()).unwrap_or(""),"albumArtist":track.and_then(|t|t.distinct_album_artist()),"album":track.map(|t|t.album_name()).unwrap_or(""),
        "autoMix":state.playback.auto_mix,"waveform":state.waveform.data().map(|data| data.resample(300)),"waveformMessage":if track.is_none(){Some("Nothing playing")}else if state.waveform.error().is_some(){Some("Waveform unavailable")}else if state.waveform.data().is_none(){Some("Preparing waveform…")}else{None},"playing":state.playback.status==PlayStatus::Playing,"position":state.playback.position_ms,"duration":state.playback.duration_ms,"volume":state.playback.volume,"artwork":artwork,"rows":track.map(app::details::now_playing_rows).unwrap_or_default()})
}

#[cfg(test)]
mod tests {
    use super::super::commands::Command;
    use super::*;

    #[test]
    fn history_removal_uses_the_swiped_entry_even_when_another_row_is_selected() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        let first = BrowseItem::Description {
            query: "Quiet piano".into(),
        };
        let second = BrowseItem::Description {
            query: "Bright strings".into(),
        };
        let col = BrowseColumn::new_with_tracks("Description history", vec![first, second], vec![]);
        let menu = row_context(&state, &mut adapter, "history", &col, 1, &col.items[1]);
        let remove = menu
            .iter()
            .find(|c| c["kind"] == "removeDescription")
            .unwrap();
        let actions = adapter.handle(
            Command::Invoke {
                token: remove["token"].as_u64().unwrap(),
            },
            &mut state,
            &Default::default(),
        );
        assert!(
            matches!(actions.as_slice(), [Action::Source(app::sources::SourceAction::AudioMuse(action))]
            if matches!(action.as_ref(), app::sources::audiomuse::Command::RemoveDescription { query } if query == "Bright strings"))
        );
    }

    #[test]
    fn search_labels_the_result_and_its_recent_entry() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        state.search.query = "example".into();
        state.search.results = Some(crate::library::models::SearchResults {
            tracks: vec![crate::library::models::Track {
                title: "Example".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let rows = search(&state, &mut adapter);
        assert_eq!(rows[0]["label"], "Example");
        assert!(rows[0]["subtitle"].as_str().unwrap().starts_with("Track"));
        state.search.query.clear();
        state
            .search
            .recent
            .push(app::library_details::RecentResult {
                label: "Example playlist".into(),
                key: "playlist".into(),
                target: app::library_details::RecentTarget::Playlist,
            });
        let rows = search(&state, &mut adapter);
        assert_eq!(rows[0]["label"], "Example playlist");
        assert_eq!(rows[0]["subtitle"], "Playlist");
    }

    #[tokio::test]
    async fn playlist_picker_filters_and_appends_the_original_selection() {
        use app::sources::navidrome::{
            NavAction, commands::Command as Server, playlist_edit::Addition,
        };
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = app::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        state.replace_playlists(
            ["Favorites", "Training"]
                .iter()
                .map(|name| {
                    session
                        .playlist(serde_json::from_value(json!({"id":name,"name":name})).unwrap())
                })
                .collect(),
        );
        let song_key = session.key("one");
        let col = BrowseColumn::new_with_tracks("Tracks", vec![], vec![]);
        let item = BrowseItem::Album {
            key: session.key("album"),
            title: "Album".into(),
            artist: "Artist".into(),
            thumb: None,
            year: None,
            is_placeholder: false,
        };
        state.sources.active = app::sources::ActiveSource::Navidrome(Box::new(session.clone()));
        let mut adapter = Adapter::default();
        adapter.begin_projection();
        let controls = row_context(&state, &mut adapter, "album-column", &col, 0, &item);
        let add = controls
            .iter()
            .find(|c| c["label"] == "Add Album to Playlist…")
            .unwrap();
        let config = crate::config::Config::default();
        let commands = adapter.handle(
            Command::Invoke {
                token: add["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        assert!(
            matches!(&commands[..],[Action::Source(app::sources::SourceAction::Navidrome(c))] if matches!(c.as_ref(),NavAction::Command(Server::ChoosePlaylist(Addition::Album(k))) if k==&session.key("album")))
        );
        let tracks = vec![
            crate::library::models::Track {
                rating_key: song_key.clone(),
                ..Default::default()
            };
            2
        ];
        state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
            "Tracks",
            BrowseItem::from_tracks(&tracks),
            tracks,
        )];
        state.browse_category = BrowseCategory::Library;
        let owner = super::super::commands::owner(&state, 0);
        let row_menu = |state: &AppState, adapter: &mut Adapter, index| {
            let col = &state.artist_nav.columns[0];
            row_context(state, adapter, &owner, col, index, &col.items[index])
        };
        let controls = row_menu(&state, &mut adapter, 0);
        let select = controls
            .iter()
            .find(|c| c["label"] == "Select Track")
            .unwrap();
        adapter.handle(
            Command::Invoke {
                token: select["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        adapter.handle(
            Command::Select {
                owner: owner.clone(),
                index: 1,
                key: song_key.clone(),
            },
            &mut state,
            &config,
        );
        assert_eq!(state.artist_nav.columns[0].selected_set.len(), 2);
        let controls = row_menu(&state, &mut adapter, 0);
        let add = controls
            .iter()
            .find(|c| c["label"] == "Add Selected Tracks to Playlist…")
            .unwrap();
        let mut actions = adapter.handle(
            Command::Invoke {
                token: add["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        let Action::Source(app::sources::SourceAction::Navidrome(command)) = actions.pop().unwrap()
        else {
            panic!("missing server action")
        };
        let NavAction::Command(Server::ChoosePlaylist(addition)) = *command else {
            panic!("missing choice")
        };
        app::sources::navidrome::playlist_edit::choose(&mut state, &session, addition);
        adapter.sync(&state);
        adapter.handle(
            Command::Palette {
                query: "Train".into(),
            },
            &mut state,
            &config,
        );
        let document = model(&state, &mut adapter, &mut Assets::default());
        let choices = document["palette"].as_array().unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0]["label"], "Training");
        let actions = adapter.handle(
            Command::Invoke {
                token: choices[0]["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        assert!(actions.iter().any(|a|matches!(a,Action::Source(app::sources::SourceAction::Navidrome(c)) if matches!(c.as_ref(),NavAction::Command(Server::Append{playlist,addition:Addition::Tracks(keys)}) if keys.as_ref()==[song_key.clone(),song_key.clone()] && playlist==&session.key("Training")))));
    }

    #[tokio::test]
    async fn streaming_picker_selects_an_explicit_bitrate_and_rejects_invalid_values() {
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active =
            app::sources::ActiveSource::Navidrome(Box::new(app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
                source,
                extensions: Default::default(),
            }));
        let mut adapter = Adapter::default();
        let document = model(&state, &mut adapter, &mut Assets::default());
        let quality = document["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "Streaming quality")
            .unwrap();
        let choices = quality["choices"].as_array().unwrap();
        assert_eq!(
            choices
                .iter()
                .map(|choice| choice["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "Original audio",
                "Up to 128 kbps",
                "Up to 192 kbps",
                "Up to 256 kbps",
                "Up to 320 kbps"
            ]
        );
        let mut config = crate::config::Config::default();
        let actions = adapter.handle(
            Command::Invoke {
                token: choices[4]["token"].as_u64().unwrap(),
            },
            &mut state,
            &config,
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Settings(SettingsAction::SetTranscodeBitrate(320))]
        ));
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        for action in actions
            .into_iter()
            .chain([SettingsAction::SetTranscodeBitrate(123).into()])
        {
            app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert_eq!(state.transcode_kbps, 320);
        assert_eq!(config.playback.transcode_kbps, 320);
        let document = model(&state, &mut adapter, &mut Assets::default());
        let quality = document["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "Streaming quality")
            .unwrap();
        assert_eq!(quality["choices"][4]["checked"], true);
        assert_eq!(quality["choices"][0]["checked"], false);
    }

    #[test]
    fn library_failure_exposes_a_scoped_retry_and_other_errors_clear_it() {
        let mut state = AppState::new();
        state.set_recoverable_error(
            "Could not load Navidrome: request timed out.".into(),
            NotificationTarget::RefreshLibrary,
        );
        let mut adapter = Adapter::default();
        let document = model(&state, &mut adapter, &mut Assets::default());
        assert_eq!(document["notice"]["actionLabel"], "Retry");
        assert!(
            !document["notice"]["message"]
                .as_str()
                .unwrap()
                .contains("F5")
        );
        let token = document["notice"]["action"].as_u64().unwrap();
        let actions = adapter.handle(
            super::super::commands::Command::Invoke { token },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [app::Action::System(SystemAction::RefreshCategory(
                RefreshCategory::Artists
            ))]
        ));
        state.advance_library_generation();
        assert!(
            adapter
                .handle(
                    super::super::commands::Command::Invoke { token },
                    &mut state,
                    &crate::config::Config::default(),
                )
                .is_empty()
        );
        state.set_error("An unrelated operation failed".into());
        let document = model(&state, &mut adapter, &mut Assets::default());
        assert!(document["notice"]["action"].is_null());
        assert!(document["notice"]["actionLabel"].is_null());
        state.clear_error();
        assert!(state.notifications.error_target.is_none());
    }

    #[test]
    fn ipad_theme_labels_are_legible_on_both_library_surfaces() {
        use app::{theme::ThemeName, theme_palette::contrast};

        for &name in ThemeName::all() {
            if name == ThemeName::System {
                continue;
            }
            let palette = theme(name);
            let rgb = |key: &str| {
                let hex = palette[key].as_str().unwrap().strip_prefix('#').unwrap();
                let [_, r, g, b] = u32::from_str_radix(hex, 16).unwrap().to_be_bytes();
                [r, g, b]
            };
            for label in ["foreground", "secondary"] {
                for surface in ["background", "surface"] {
                    assert!(
                        contrast(rgb(label), rgb(surface)) >= 4.5,
                        "{name:?}: {label} on {surface}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn pending_download_notification_opens_list_and_cancel_token_removes_only_job() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("song.wav"), b"fixture audio").unwrap();
        let mut state = AppState::new();
        state.active_library = Some("fixture".into());
        state.sources.folders.push(crate::library::FolderSource {
            id: "fixture".into(),
            name: "Fixture".into(),
            location: crate::library::FolderLocation::Local {
                path: directory.path().into(),
            },
        });
        state.downloads.store =
            crate::services::downloads::Store::load(directory.path().join("downloads")).unwrap();
        let track = crate::library::models::Track::from_folder("fixture", "song.wav");
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        app::downloads::dispatch(
            app::downloads::Command::Tracks(vec![track]),
            &mut state,
            &tx,
        )
        .await
        .unwrap();
        let mut adapter = Adapter::default();
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let document = model(&state, &mut adapter, &mut Assets::default());
        let token = document["notice"]["action"].as_u64().unwrap();
        for action in adapter.handle(Command::Invoke { token }, &mut state, &config) {
            app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        let document = model(&state, &mut adapter, &mut Assets::default());
        let row = &document["columns"][0]["rows"][0];
        assert!(row["pending"].as_str().unwrap().starts_with("Downloading"));
        assert_eq!(row["context"].as_array().unwrap().len(), 1);
        assert_eq!(row["context"][0]["label"], "Cancel Download");
        assert!(
            document["columns"][0]["playback"]
                .as_array()
                .is_none_or(|a| a.is_empty())
        );
        let token = row["cancel"]["token"].as_u64().unwrap();
        for action in adapter.handle(Command::Invoke { token }, &mut state, &config) {
            app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert!(state.downloads.jobs.is_empty());
        assert!(state.artist_nav.columns[0].items.is_empty());
    }

    #[tokio::test]
    async fn downloads_palette_navigation_exposes_committed_size_in_its_column() {
        let mut state = AppState::new();
        let track = crate::library::models::Track::from_folder("fixture", "song.wav");
        state.downloads.store.entries.insert(
            crate::services::downloads::Store::id("fixture", &track),
            crate::services::downloads::Entry {
                library: "fixture".into(),
                library_name: "Fixture".into(),
                track,
                bytes: 5 * 1024 * 1024,
                artwork: false,
            },
        );
        state.track_pane_open = true;
        app::command_palette::open(&mut state);
        let mut adapter = Adapter::default();
        let token = adapter.bind(
            &state,
            "download-open",
            Binding::Palette(PaletteCommandKind::Downloads(app::downloads::Menu::Open)),
        );
        let mut config = crate::config::Config::default();
        let actions = adapter.handle(Command::Invoke { token }, &mut state, &config);
        assert!(!actions.is_empty());
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        for action in actions {
            adapter.observe(&action, &mut state);
            app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        let document = model(&state, &mut adapter, &mut Assets::default());
        assert_eq!(document["columns"][0]["title"], "Downloads");
        assert_eq!(document["columns"][0]["namedDestination"], false);
        assert_eq!(document["columns"][0]["summary"], "1 item · 5.0 MB");
        assert_eq!(document["sheet"], serde_json::Value::Null);
        assert!(!state.track_pane_open);
        let entry = &document["columns"][0]["rows"][0]["context"][0];
        assert_eq!(entry["label"], "Remove Download");
        let token = entry["token"].as_u64().unwrap();
        let actions = adapter.handle(Command::Invoke { token }, &mut state, &config);
        assert!(matches!(
            actions.as_slice(),
            [Action::System(SystemAction::Downloads(
                app::downloads::Command::Remove(_)
            ))]
        ));
    }

    #[test]
    fn album_headers_share_artwork_without_repeating_it_on_tracks_but_playlists_show_covers() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.category_column_focused = false;
        state.library_navigation = app::navigation_history::LibraryNavigation::Columns(None);
        let track = crate::library::models::Track {
            rating_key: "track".into(),
            title: "Track".into(),
            parent_rating_key: Some("album".into()),
            parent_thumb: Some("cover".into()),
            ..Default::default()
        };
        let mut column = BrowseColumn::new_with_tracks(
            "Album",
            BrowseItem::from_tracks(std::slice::from_ref(&track)),
            vec![track],
        );
        column.play_all_row = Some(PlayAllRow::Album {
            rating_key: "album".into(),
            title: "Album".into(),
        });
        state.initialize_column_artwork(&mut column);
        state.artist_nav.columns = vec![column];
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"][0]["namedDestination"], true);
        assert!(snapshot["columns"][0]["albumDetailsArtwork"].is_string());
        assert!(snapshot["columns"][0]["rows"][0]["artwork"].is_null());
        state.artist_nav.columns[0].play_all_row = Some(PlayAllRow::Playlist {
            rating_key: "playlist".into(),
            title: "Playlist".into(),
        });
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"][0]["namedDestination"], true);
        assert!(snapshot["columns"][0]["rows"][0]["artwork"].is_string());
        state.artist_nav.columns[0].artwork_visible = false;
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert!(snapshot["columns"][0]["rows"][0]["artwork"].is_null());
    }

    #[test]
    fn external_search_availability_and_execution_follow_the_visible_music() {
        let mut state = AppState::new();
        state.view = View::Browse;
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        app::command_palette::open(&mut state);
        let empty = model(&state, &mut adapter, &mut assets);
        let search = &empty["externalSearches"][0];
        assert_eq!(search["label"], "Search Apple Music");
        assert!(search["disabled"].is_string());
        assert!(
            adapter
                .handle(
                    Command::Invoke {
                        token: search["token"].as_u64().unwrap()
                    },
                    &mut state,
                    &Default::default()
                )
                .is_empty()
        );
        assert!(state.palette.open);
        assert!(state.popups.input_dialog().is_none());
        state.category_column_focused = false;
        state.artist_nav.columns = vec![BrowseColumn::new("Artists", vec![artist("Momus")])];
        let selected = model(&state, &mut adapter, &mut assets);
        let search = &selected["externalSearches"][0];
        assert!(search["disabled"].is_null());
        let actions = adapter.handle(
            Command::Invoke {
                token: search["token"].as_u64().unwrap(),
            },
            &mut state,
            &Default::default(),
        );
        assert!(
            matches!(actions.as_slice(), [Action::System(SystemAction::OpenExternalSearch { query: Some(query), .. })] if query.text() == "Momus")
        );
        assert!(state.popups.input_dialog().is_none());
    }

    #[test]
    fn long_columns_get_an_index_and_short_columns_leave_the_edge_for_scrolling() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        state.library_navigation = super::super::commands::LibraryNavigation::Columns(None);
        let mut assets = Assets::default();
        state.artist_nav.columns = vec![BrowseColumn::new(
            "Artists",
            (0..270).map(|i| artist(&format!("Artist {i}"))).collect(),
        )];
        assert!(
            model(&state, &mut adapter, &mut assets)["columns"][0]["sections"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        state.artist_nav.columns[0].items.push(artist("Z Artist"));
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"][0]["sections"][1]["label"], "Z");
        assert_eq!(snapshot["columns"][0]["sections"][1]["position"], 270);
    }

    #[test]
    fn category_filter_is_owned_by_its_column_and_library() {
        use app::sidebar::SidebarGroup;
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        state.library_navigation =
            super::super::commands::LibraryNavigation::Group(SidebarGroup::Albums);
        let mut assets = Assets::default();
        let owner = super::super::commands::group_owner(&state, SidebarGroup::Albums);
        adapter.handle(
            Command::Filter {
                owner: owner.clone(),
                query: "Singles".into(),
            },
            &mut state,
            &Default::default(),
        );
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"][0]["count"], 1);
        assert_eq!(snapshot["columns"][0]["rows"][0]["title"], "Singles");
        assert!(!state.list_filter.active);
        state.library_generation += 1;
        adapter.handle(
            Command::Filter {
                owner,
                query: "All".into(),
            },
            &mut state,
            &Default::default(),
        );
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"][0]["filter"], "");
        assert!(snapshot["columns"][0]["count"].as_u64().unwrap() > 1);
        assert!(adapter.group_filters.is_empty());
    }

    #[test]
    fn sidebar_group_retains_its_parent_when_opening_a_destination() {
        use super::super::commands::LibraryNavigation;
        use app::sidebar::SidebarGroup;
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let snapshot = model(&state, &mut adapter, &mut assets);
        let albums = snapshot["sidebarGroups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["label"] == "Albums")
            .unwrap();
        let token = albums["token"].as_u64().unwrap();
        assert!(
            adapter
                .handle(Command::Invoke { token }, &mut state, &Default::default())
                .is_empty()
        );
        let snapshot = model(&state, &mut adapter, &mut assets);
        let columns = snapshot["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0]["title"], "Albums");
        let all = columns[0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["title"] == "All")
            .unwrap();
        let actions = adapter.handle(
            Command::Invoke {
                token: all["activation"].as_u64().unwrap(),
            },
            &mut state,
            &Default::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Miller(MillerAction::LoadAllAlbumsForMiller { .. })]
        ));
        assert_eq!(
            state.library_navigation,
            LibraryNavigation::Columns(Some(SidebarGroup::Albums))
        );
        adapter.handle(Command::Pop { depth: 1 }, &mut state, &Default::default());
        assert_eq!(
            model(&state, &mut adapter, &mut assets)["columns"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        adapter.handle(Command::Pop { depth: 0 }, &mut state, &Default::default());
        assert!(
            model(&state, &mut adapter, &mut assets)["columns"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        state.library_generation += 1;
        adapter.handle(Command::Invoke { token }, &mut state, &Default::default());
        assert_eq!(state.library_navigation, LibraryNavigation::Home);
    }

    fn artist(title: &str) -> BrowseItem {
        BrowseItem::Artist {
            key: title.into(),
            title: title.into(),
            thumb: None,
            is_placeholder: false,
        }
    }

    #[test]
    fn command_and_same_named_submenu_keep_independent_bindings() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        app::command_palette::open_with_query(&mut state, "Play Queue");
        let command = PaletteCommandKind::GotoQueue;
        let group = PaletteCommandKind::OpenCommandGroup(vec!["Play Queue".into()]);
        let open = adapter.bind(
            &state,
            &palette_identity(&command, "Play Queue"),
            Binding::Palette(command),
        );
        let expand = adapter.bind(
            &state,
            &palette_identity(&group, "Play Queue"),
            Binding::Palette(group),
        );
        assert_ne!(open, expand);
        let actions = adapter.handle(
            Command::Invoke { token: open },
            &mut state,
            &Default::default(),
        );
        assert!(!state.palette.open);
        assert!(actions.iter().any(|action| matches!(
            action,
            Action::Navigation(NavigationAction::SetView(View::Queue))
        )));
        app::command_palette::open(&mut state);
        adapter.handle(
            Command::Invoke { token: expand },
            &mut state,
            &Default::default(),
        );
        assert!(state.palette.open);
    }

    #[test]
    fn mobile_palette_drills_menu_groups_and_restores_the_parent() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        app::command_palette::open(&mut state);
        let mut snapshot = model(&state, &mut adapter, &mut assets);
        for (label, path) in [("View", vec!["View"]), ("Browse", vec!["View", "Browse"])] {
            let token = snapshot["palette"]
                .as_array()
                .unwrap()
                .iter()
                .find(|control| control["label"] == label)
                .unwrap()["token"]
                .as_u64()
                .unwrap();
            assert!(
                adapter
                    .handle(Command::Invoke { token }, &mut state, &Default::default())
                    .is_empty()
            );
            snapshot = model(&state, &mut adapter, &mut assets);
            assert_eq!(snapshot["palettePath"], json!(path));
        }
        assert!(
            snapshot["palette"]
                .as_array()
                .unwrap()
                .iter()
                .any(|control| control["label"] == "Play Queue")
        );
        let token = snapshot["paletteBack"]["token"].as_u64().unwrap();
        adapter.handle(Command::Invoke { token }, &mut state, &Default::default());
        assert_eq!(state.palette.path, ["View"]);
        assert!(state.palette.open);
    }

    #[test]
    fn mobile_sidebar_preferences_and_singles_share_core_visibility() {
        let mut state = AppState::new();
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let first = model(&state, &mut adapter, &mut assets);
        let settings = first["settings"].as_array().unwrap();
        let singles = settings
            .iter()
            .find(|setting| setting["label"] == "Singles: maximum tracks")
            .unwrap();
        assert_eq!(
            singles["bounds"],
            json!([2, 1, crate::services::singles::MAX_LIMIT])
        );
        let folders = settings
            .iter()
            .find(|setting| setting["label"] == "Folders")
            .unwrap();
        let actions = adapter.handle(
            Command::Invoke {
                token: folders["token"].as_u64().unwrap(),
            },
            &mut state,
            &Default::default(),
        );
        assert!(matches!(
            actions.as_slice(),
            [Action::Settings(SettingsAction::ActivatePreference(_))]
        ));
        state.hidden_sections.push(BrowseCategory::Folders);
        let hidden = model(&state, &mut adapter, &mut assets);
        assert!(
            !hidden["roots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|root| root["label"] == "Folders")
        );
    }

    #[test]
    fn alphabet_index_uses_catalog_order_articles_and_filtered_positions() {
        let items = [
            BrowseItem::AllArtists,
            artist("123"),
            artist("A Singer"),
            artist("The Beatles"),
            artist("Z Artist"),
            artist("文"),
        ];
        let index = alphabet_sections(items.iter());
        assert_eq!(
            index
                .iter()
                .map(|entry| entry["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["", "0", "A", "B", "Z", "文"]
        );
        assert_eq!(index[3]["position"], 3);
        let filtered = alphabet_sections([&items[2], &items[4]].into_iter());
        assert_eq!(filtered[1]["position"], 1);
        let mixed =
            alphabet_sections([artist("B"), artist("A"), artist("Z"), artist("B again")].iter());
        assert_eq!(mixed.len(), 3);
        assert_eq!(mixed[1]["label"], "A");
        assert_eq!(mixed[1]["position"], 1);
    }

    #[test]
    fn scrollbar_windows_cover_distant_rows_without_serializing_the_whole_catalog() {
        let mut state = AppState::new();
        state.artist_nav.columns = vec![BrowseColumn::new(
            "Artists",
            (0..22_000)
                .map(|index| artist(&format!("Artist {index:05}")))
                .collect(),
        )];
        let mut adapter = Adapter::default();
        state.library_navigation = super::super::commands::LibraryNavigation::Columns(None);
        let config = crate::config::Config::default();
        let mut assets = Assets::default();
        let owner = super::super::commands::owner(&state, 0);
        adapter.handle(
            Command::Viewport {
                owner: owner.clone(),
                start: 21_900,
                end: 21_910,
            },
            &mut state,
            &config,
        );
        let first = model(&state, &mut adapter, &mut assets);
        let column = &first["columns"][0];
        assert_eq!(column["count"], 22_000);
        let rows = column["rows"].as_array().unwrap();
        assert!(rows.len() <= 300);
        assert!(rows.iter().any(|row| row["title"] == "Artist 21900"));
        assert_eq!(state.artist_nav.columns[0].selected_index, 0);
        state.library_generation += 1;
        assert!(
            adapter
                .handle(
                    Command::Viewport {
                        owner,
                        start: 100,
                        end: 120
                    },
                    &mut state,
                    &config
                )
                .is_empty()
        );
        assert!(
            !adapter
                .windows
                .contains_key(&super::super::commands::owner(&state, 0))
        );
    }

    #[test]
    fn filtered_folders_keep_display_positions_separate_from_source_indices() {
        use crate::library::folder::{FolderColumn, FolderItem, FolderNavigationState};
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Folders;
        state.folder_state = Some(FolderNavigationState::with_root(
            "library".into(),
            FolderColumn::new(
                None,
                "Folders".into(),
                (0..10)
                    .map(|index| FolderItem::folder(index.to_string(), format!("Folder {index}")))
                    .collect(),
            ),
        ));
        state.list_filter.active = true;
        state.list_filter.category = BrowseCategory::Folders;
        state.list_filter.query = "Folder".into();
        state.list_filter.results = Some(crate::services::ListFilterResults {
            matched_indices: vec![3, 8],
            total_matches: 2,
            has_more: false,
        });
        let mut adapter = Adapter::default();
        state.library_navigation = super::super::commands::LibraryNavigation::Columns(None);
        adapter.handle(
            Command::Sheet {
                sheet: Some(super::super::commands::Sheet::Search),
            },
            &mut state,
            &crate::config::Config::default(),
        );
        let document = model(&state, &mut adapter, &mut Assets::default());
        let column = &document["columns"][0];
        assert_eq!(column["count"], 2);
        assert_eq!(column["filter"], "Folder");
        assert_eq!(column["rows"][0]["position"], 0);
        assert_eq!(column["rows"][0]["index"], 3);
        assert_eq!(column["rows"][1]["position"], 1);
        assert_eq!(column["rows"][1]["index"], 8);
        let closing = adapter.handle(
            Command::Sheet { sheet: None },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(
            !closing
                .iter()
                .any(|action| matches!(action, Action::Navigation(NavigationAction::SetView(_))))
        );
    }

    #[tokio::test]
    async fn queue_type_seed_choice_opens_inline_station_configuration() {
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "test".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active =
            app::sources::ActiveSource::Navidrome(Box::new(app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
                source,
                extensions: Default::default(),
            }));
        state.active_library = Some("fixture".into());
        state.station_nav.columns = vec![StationColumn::new(
            None,
            "Radio".into(),
            crate::library::station::standard_stations(
                "nav-radio",
                crate::library::capabilities::NAVIDROME,
            ),
        )];
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let document = model(&state, &mut adapter, &mut assets);
        let choice = document["queueModes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|choice| choice["label"] == "Stay Close Mix")
            .unwrap();
        let actions = adapter.handle(
            Command::Invoke {
                token: choice["token"].as_u64().unwrap(),
            },
            &mut state,
            &crate::config::Config::default(),
        );
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let mut config = crate::config::Config::default();
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        for action in actions {
            crate::app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        let document = model(&state, &mut adapter, &mut assets);
        assert_eq!(document["sheet"], serde_json::Value::Null);
        assert_eq!(
            document["columns"].as_array().unwrap().last().unwrap()["stationDetails"]["input"],
            "track"
        );
        assert!(state.jump_history.head().is_some());
    }

    #[test]
    fn radio_row_taps_play_leaves_but_arrows_and_required_choices_open_columns() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.desktop_library_root = LibraryRoot::Radio;
        state.library_navigation = app::navigation_history::LibraryNavigation::Columns(None);
        state.station_nav.columns = vec![StationColumn::new(
            None,
            "Radio".into(),
            crate::library::station::standard_stations(
                "nav-radio",
                crate::library::capabilities::NAVIDROME,
            ),
        )];
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let document = model(&state, &mut adapter, &mut assets);
        let group = document["sidebarGroups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["label"] == "Radio")
            .unwrap();
        state.library_navigation = app::navigation_history::LibraryNavigation::Home;
        adapter.handle(
            Command::Invoke {
                token: group["token"].as_u64().unwrap(),
            },
            &mut state,
            &crate::config::Config::default(),
        );
        let document = model(&state, &mut adapter, &mut assets);
        assert_eq!(document["columns"].as_array().unwrap().len(), 1);
        assert_eq!(document["columns"][0]["radio"], true);
        assert!(
            document["roots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["section"] == "Radio" && item["label"] == "Play Random Albums")
        );
        let row = document["columns"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"] == "nav-radio/randomAlbum")
            .unwrap();
        let play = row["activation"].as_u64().unwrap();
        let open = row["disclosure"].as_u64().unwrap();
        let actions = adapter.handle(
            Command::Invoke { token: play },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(
            matches!(actions.as_slice(),[Action::Radio(RadioAction::PlayStation(key))] if key=="nav-radio/randomAlbum")
        );
        assert!(
            adapter
                .handle(
                    Command::Invoke { token: open },
                    &mut state,
                    &crate::config::Config::default()
                )
                .is_empty()
        );
        let document = model(&state, &mut adapter, &mut assets);
        assert!(
            document["columns"].as_array().unwrap().last().unwrap()["stationDetails"]["text"]
                .as_str()
                .unwrap()
                .contains("randomly chosen album")
        );
        let row = document["columns"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"] == "nav-radio/decade")
            .unwrap();
        let actions = adapter.handle(
            Command::Invoke {
                token: row["activation"].as_u64().unwrap(),
            },
            &mut state,
            &crate::config::Config::default(),
        );
        assert!(
            matches!(actions.as_slice(),[Action::Radio(RadioAction::DrillIntoStation(key,_))] if key=="nav-radio/decade")
        );
    }

    #[test]
    fn radio_queue_preserves_order_while_the_playing_marker_advances() {
        let mut state = AppState::new();
        state.playback_mode = PlaybackMode::Radio;
        state.queue.index = Some(0);
        state.radio.tracks = (0..3)
            .map(|index| crate::library::models::Track {
                rating_key: format!("track-{index}"),
                ..Default::default()
            })
            .collect();
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        state.radio.track_index = Some(0);
        let before = model(&state, &mut adapter, &mut assets);
        state.radio.track_index = Some(2);
        let after = model(&state, &mut adapter, &mut assets);
        for index in 0..3 {
            assert_eq!(before["queue"][index]["id"], after["queue"][index]["id"]);
        }
        assert_eq!(after["queue"][0]["selected"], false);
        assert_eq!(after["queue"][2]["selected"], true);
    }
}

#[cfg(test)]
mod album_parent_tests {
    use super::*;
    use crate::app::navigation_history::LibraryNavigation;
    use crate::app::sidebar::SidebarGroup;
    #[test]
    fn compilations_back_returns_to_albums_without_exposing_the_internal_artist_root() {
        let mut state = AppState::new();
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.desktop_library_root = LibraryRoot::Compilations;
        state.library_navigation = LibraryNavigation::Columns(Some(SidebarGroup::Albums));
        state.artist_nav.columns = vec![
            BrowseColumn::new("artists", vec![]),
            BrowseColumn::new("Compilations", vec![]),
        ];
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let snapshot = model(&state, &mut adapter, &mut assets);
        let titles: Vec<_> = snapshot["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|column| column["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["Albums", "Compilations"]);
        adapter.handle(
            super::super::commands::Command::Pop { depth: 1 },
            &mut state,
            &Default::default(),
        );
        let snapshot = model(&state, &mut adapter, &mut assets);
        assert_eq!(snapshot["columns"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["columns"][0]["title"], "Albums");
        assert_eq!(
            state.library_navigation,
            LibraryNavigation::Group(SidebarGroup::Albums)
        );
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[test]
    fn asset_identity_changes_with_content_and_cannot_outlive_a_cache_clear() {
        let mut state = AppState::new();
        let mut assets = Assets::default();
        state.artwork.insert_grid_art("album".into(), vec![1, 2, 3]);
        let original = assets.artwork(&state, "album", "album");
        assert_eq!(assets.artwork(&state, "album", "album"), original);
        state.artwork.insert_grid_art("album".into(), vec![3, 2, 1]);
        let updated = assets.artwork(&state, "album", "album");
        assert_ne!(updated, original);
        assert_eq!(assets.bytes.get(&updated).unwrap().as_slice(), [3, 2, 1]);
        state.artwork.clear_grid_art();
        assert_eq!(
            assets.artwork(&state, "album", "album"),
            "placeholder-album"
        );
    }
    #[tokio::test]
    async fn navidrome_root_waits_for_browse_catalog_but_not_full_track_refresh() {
        let mut state = AppState::new();
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let session = app::sources::navidrome::Session {
            client: crate::navidrome::Client::new(&source, "fixture".into(), None).unwrap(),
            source,
            extensions: Default::default(),
        };
        let playlist =
            session.playlist(serde_json::from_value(json!({"id":"p","name":"Favorites"})).unwrap());
        state.sources.active = app::sources::ActiveSource::Navidrome(Box::new(session));
        state.library.catalog_loading = app::catalog_loading::State::Browsing { pending: None };
        state.library_loading = true;
        state.library.artists_loading = true;
        let mut adapter = Adapter::default();
        let mut assets = Assets::default();
        let opening = model(&state, &mut adapter, &mut assets);
        assert_eq!(opening["libraryLoading"], true);
        assert_eq!(opening["roots"], json!([]));
        assert_eq!(opening["sidebarGroups"], json!([]));

        state.replace_playlists(vec![playlist]);
        state.library.artists_loading = false;
        let browsing = model(&state, &mut adapter, &mut assets);
        assert_eq!(browsing["libraryLoading"], false);
        assert!(
            browsing["sidebarGroups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["label"] == "Playlists")
        );

        state.library.catalog_loading = app::catalog_loading::State::Ready;
        state.library.artists_loading = true;
        assert!(!library_loading(&state));
    }

    #[test]
    fn active_source_exposes_navigation_while_catalog_loads() {
        let mut state = AppState::new();
        assert!(!library_loading(&state));
        state.library_loading = true;
        assert!(library_loading(&state));
        state.sources.active = app::sources::ActiveSource::Folder("fixture".into());
        state.library.artists_loading = true;
        assert!(!library_loading(&state));
        state.library.artists_loading = false;
        state.library.catalog_loading = app::catalog_loading::State::Browsing { pending: None };
        assert!(!library_loading(&state));
    }
}
