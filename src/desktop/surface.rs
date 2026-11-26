//! Native presentation of the existing app dialogs. The engine owns all reducers.
use crate::app::sources::dialogs::Dialog as LibraryDialog;
use crate::app::{Action, AppState, action::*, state::*};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
pub enum Operation {
    Native(super::menu_catalog::Native),
    Key(KeyCode),
    Actions(Vec<Action>),
    Row(usize),
    ViewOption(SortPopupOption),
    RadioSelect { column: usize, key: String },
    Tab(usize),
    Palette(PaletteCommandKind),
    Confirm(bool),
    AdventureStep(AdventureStep),
    AdventureReverse,
    AdventureGenerate,
    LibraryPage(crate::app::sources::options::OptionsPage),
}
#[derive(Clone)]
pub struct Item {
    pub glyph: Option<gpui_kit::assets::IconName>,
    pub key: String,
    pub label: String,
    pub selected: bool,
    pub aliases: Vec<String>,
    pub operation: Operation,
}
#[derive(Clone)]
pub struct Field {
    pub label: String,
    pub value: String,
    pub masked: bool,
}
#[derive(Clone, Copy)]
pub enum ChoiceControl {
    Checkbox,
    Radio,
}
#[derive(Clone)]
pub struct Toggle {
    pub disabled: bool,
    pub control: ChoiceControl,
    pub label: String,
    pub checked: bool,
    pub operation: Operation,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum WindowLayout {
    #[default]
    Utility,
    Browser,
    Reading,
}
#[derive(Clone)]
pub struct HelpSection {
    pub title: &'static str,
    pub rows: &'static [(&'static str, &'static str)],
}
#[derive(Clone)]
pub enum PlaylistFile {
    Import {
        generation: u64,
    },
    Export {
        suggested_name: String,
        action: Action,
    },
}
#[derive(Clone, Default)]
pub struct Surface {
    pub id: String,
    pub revision: u64,
    pub title: String,
    pub layout: WindowLayout,
    pub fixed_size: bool,
    pub library_inspector: bool,
    pub player_reader: bool,
    pub playlist_file: Option<PlaylistFile>,
    pub help: Vec<HelpSection>,
    context_key: String,
    pub text: String,
    pub endpoints: Vec<Item>,
    pub loading: bool,
    pub fields: Vec<Field>,
    pub form_hidden: bool,
    pub multiple_choice: bool,
    pub selected_names: Vec<String>,
    pub examples: Vec<String>,
    pub toggles: Vec<Toggle>,
    pub rows: Vec<Item>,
    pub disabled_rows: std::collections::BTreeMap<usize, &'static str>,
    pub buttons: Vec<Item>,
    pub primary: Option<String>,
    pub primary_disabled: bool,
    pub tabs: Vec<Item>,
    pub palette: bool,
    pub palette_path: Vec<String>,
    pub stale: bool,
    pub radio: Option<super::radio::Model>,
    pub lyrics: Option<crate::app::lyrics::View>,
    pub query: String,
    pub bio_search: Option<crate::services::web_search::Provider>,
    pub image: Option<Vec<u8>>,
}
impl Surface {
    fn new(key: &str, state: &AppState, title: impl Into<String>) -> Self {
        Self {
            id: format!("{}:{key}", state.library_generation),
            title: title.into(),
            loading: crate::app::activity::message(state).is_some(),
            context_key: state
                .palette_target_track()
                .map(|t| t.rating_key)
                .or_else(|| state.focused_album().map(|(key, _)| key))
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    fn button(&mut self, label: impl Into<String>, operation: Operation) {
        let label = label.into();
        self.buttons.push(Item {
            glyph: None,
            aliases: vec![],
            key: label.clone(),
            label,
            selected: false,
            operation,
        });
    }
    fn primary_button(&mut self, label: impl Into<String>, operation: Operation) {
        let label = label.into();
        self.primary = Some(label.clone());
        self.button(label, operation);
    }
    fn row(
        &mut self,
        key: impl Into<String>,
        label: impl Into<String>,
        selected: bool,
        index: usize,
    ) {
        self.rows.push(Item {
            glyph: None,
            aliases: vec![],
            key: key.into(),
            label: label.into(),
            selected,
            operation: Operation::Row(index),
        });
    }
    fn field(&mut self, label: impl Into<String>, value: impl Into<String>, masked: bool) {
        self.fields.push(Field {
            label: label.into(),
            value: if masked { String::new() } else { value.into() },
            masked,
        });
    }
    fn finish(mut self) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.id.hash(&mut h);
        if self.library_inspector {
            self.text.hash(&mut h);
        }
        self.palette_path.hash(&mut h);
        self.disabled_rows.hash(&mut h);
        self.context_key.hash(&mut h);
        if let Some(radio) = &self.radio {
            radio.revision().hash(&mut h);
        }
        for item in self
            .rows
            .iter()
            .chain(&self.buttons)
            .chain(&self.tabs)
            .chain(&self.endpoints)
        {
            item.glyph.hash(&mut h);
            item.key.hash(&mut h);
            item.aliases.hash(&mut h);
            item.label.hash(&mut h);
            item.selected.hash(&mut h);
        }
        for toggle in &self.toggles {
            toggle.label.hash(&mut h);
            toggle.checked.hash(&mut h);
            toggle.disabled.hash(&mut h);
        }
        if let Some(lyrics) = &self.lyrics {
            lyrics.active.hash(&mut h);
            lyrics.follow.hash(&mut h);
            lyrics.language.hash(&mut h);
            lyrics.message.hash(&mut h);
            for line in &lyrics.lines {
                line.value.hash(&mut h);
                for span in &line.spans {
                    span.active.hash(&mut h);
                    span.text.hash(&mut h);
                }
            }
        }
        self.revision = h.finish();
        self
    }
}
fn result_rows(
    results: &crate::library::models::SearchResults,
    tab: SearchTab,
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if matches!(tab, SearchTab::Global | SearchTab::Artists) {
        rows.extend(
            results
                .artists
                .iter()
                .map(|a| (a.rating_key.clone(), format!("Artist · {}", a.title))),
        );
    }
    if matches!(tab, SearchTab::Global | SearchTab::Albums) {
        rows.extend(
            results
                .albums
                .iter()
                .map(|a| (a.rating_key.clone(), format!("Album · {}", a.title))),
        );
    }
    if matches!(tab, SearchTab::Global | SearchTab::Playlists) {
        rows.extend(
            results
                .playlists
                .iter()
                .map(|p| (p.rating_key.clone(), format!("Playlist · {}", p.title))),
        );
    }
    if matches!(tab, SearchTab::Global | SearchTab::Genres) {
        rows.extend(
            results
                .genres
                .iter()
                .map(|g| (g.key.clone(), format!("Genre · {}", g.title))),
        );
    }
    if matches!(tab, SearchTab::Global | SearchTab::Tracks) {
        rows.extend(results.tracks.iter().map(|t| {
            (
                t.rating_key.clone(),
                format!(
                    "{}\nTrack · {} · {}",
                    t.title,
                    t.track_artist(),
                    t.album_name()
                ),
            )
        }));
    }
    if tab == SearchTab::Global && !results.global_order.is_empty() {
        return results
            .global_order
            .iter()
            .map(|&i| std::mem::take(&mut rows[i]))
            .collect();
    }
    rows
}
fn tabs(model: &mut Surface, selected: SearchTab) {
    for (index, tab) in SearchTab::all().iter().enumerate() {
        model.tabs.push(Item {
            glyph: None,
            aliases: vec![],
            key: tab.name().into(),
            label: tab.name().into(),
            selected: *tab == selected,
            operation: Operation::Tab(index),
        });
    }
}

pub(super) fn radio_surface(state: &AppState) -> Surface {
    let mut model = Surface::new("radio-browser", state, "Radio");
    model.layout = WindowLayout::Browser;
    model.radio = Some(super::radio::project(state));
    if state.playback_mode == PlaybackMode::Radio {
        model.button(
            "Manual Queue",
            Operation::Actions(vec![crate::app::action::RadioAction::StopStation.into()]),
        );
    }
    model.finish()
}

pub(super) fn unavailable_description(state: &AppState) -> Surface {
    let mut model = Surface::new("describe-unavailable", state, "Describe Music");
    model.text="Describe Music needs AudioMuse. Enable sonic features and connect AudioMuse in Settings → Libraries.".into();
    model.button(
        "Settings…",
        Operation::Palette(PaletteCommandKind::OpenSettings),
    );
    model.finish()
}

pub fn project(state: &AppState) -> Option<Surface> {
    if state.palette.open {
        let mut m = Surface::new(
            &format!("palette:{}", state.station_navigation_generation),
            state,
            "Command Palette",
        );
        m.palette = true;
        m.palette_path = state.palette.path.clone();
        if !m.palette_path.is_empty() {
            m.title = format!("Command Palette — {}", m.palette_path.join(" › "));
        }
        m.query = state.palette.query.clone();
        let entries = crate::app::sources::navidrome::playlist_edit::choices(state)
            .unwrap_or_else(|| crate::app::command_palette::materialize_entries(state));
        for mut entry in entries {
            crate::app::command_catalog::prepare(&mut entry);
            if let Some(reason) =
                crate::app::command_catalog::disabled_reason(state, &entry.command)
            {
                m.disabled_rows.insert(m.rows.len(), reason);
            }
            // Window and layout commands are handled by the desktop adapter.
            m.rows.push(Item {
                glyph: None,
                aliases: entry.aliases,
                key: entry.label.clone(),
                label: entry.label,
                selected: false,
                operation: Operation::Palette(entry.command),
            });
        }
        if state.palette.path == [crate::app::sources::navidrome::playlist_edit::PICKER] {
            m.title = "Add to Playlist".into();
            return Some(m.finish());
        }
        m.rows.extend(super::menu_catalog::native_entries(
            &super::menu_catalog::menus(&super::menu_catalog::catalog(Some(state))),
        ));
        super::palette::append_groups(&mut m.rows, &m.palette_path);
        return Some(m.finish());
    }
    content(state)
}

pub(super) fn content(state: &AppState) -> Option<Surface> {
    let m = match state.popups.modal() {
        Modal::Closed => match state.view {
            View::Search => search(state),
            View::Help => {
                let mut m = Surface::new("help", state, "Textamp Help");
                m.layout = WindowLayout::Reading;
                m.help = vec![
                    HelpSection {
                        title: "Browsing",
                        rows: &[
                            (
                                "Click",
                                "Select an item and open its child column or details.",
                            ),
                            ("Double-click", "Play the selected music."),
                            ("↑ / ↓", "Move through the current column."),
                            ("← / →", "Move between parent and child columns."),
                            (
                                "Return / →",
                                "Open the selected item or move into its child column.",
                            ),
                            ("⌘Return", "Play the selected track and following tracks."),
                            ("A–Z", "Go to a name’s first letter."),
                            (
                                "⇧A–Z",
                                "Go by the second letter within the current alphabet section.",
                            ),
                            ("# / %", "Go to numbers or symbols."),
                            (
                                "Fast scrolling",
                                "Show alphabet shortcuts; the neighboring scrollbar scrolls the main list.",
                            ),
                            ("⌘-click", "Toggle an item in a multiple selection."),
                            ("⇧-click", "Select a range of tracks."),
                            (
                                "Right-click",
                                "Open actions for the clicked item or selection.",
                            ),
                            ("⌘F", "Search your library."),
                            ("/", "Filter the current column."),
                            ("⌘G", "Browse genres."),
                            ("⇧⌘O", "Browse folders."),
                            ("F6", "Open the current column’s View Options."),
                            ("⌘[", "Close the current column."),
                        ],
                    },
                    HelpSection {
                        title: "Playback & Queue",
                        rows: &[
                            ("Space", "Play or pause."),
                            ("< / >", "Previous or next track."),
                            ("⇧← / ⇧→", "Seek backward or forward."),
                            ("⇧⌘↑ / ⇧⌘↓", "Raise or lower the volume."),
                            ("⌘E", "Add the selection to the queue."),
                            ("⇧⌘E", "Insert the selection after the current track."),
                            ("⌘S", "Save the queue as a playlist."),
                            ("Delete", "Remove selected queue entries."),
                            ("⌘X", "Clear the queue."),
                            ("⌘Z", "Undo a queue edit."),
                            (
                                "Drag tracks",
                                "Reorder a track or a selected group in the queue.",
                            ),
                            ("⇧↑ / ⇧↓", "Move selected queue entries up or down."),
                            (
                                "Radio",
                                "Choose continuous playback from the queue’s Radio menu.",
                            ),
                            (
                                "Remix Tools",
                                "Use the queue’s remix icons; hover for a brief description.",
                            ),
                            ("1–6", "Choose a visualizer in Now Playing."),
                            (
                                "F7",
                                "Expand or collapse the visualizer; Escape also exits it.",
                            ),
                        ],
                    },
                    HelpSection {
                        title: "Windows & Settings",
                        rows: &[
                            ("⌘1 / ⌘L", "Show the library."),
                            (
                                "Window menu",
                                "Use a single window or open a separate Now Playing window.",
                            ),
                            (
                                "⌘3 / ⌘N",
                                "Show Now Playing; resize smaller for the mini-player.",
                            ),
                            ("⌘,", "Open Settings in its own window."),
                            (": / ⇧⌘P", "Open the command palette from any window."),
                            (
                                "Escape",
                                "Dismiss the command palette or a workflow window.",
                            ),
                            ("F1", "Open Textamp Help."),
                            ("F5", "Refresh the library."),
                            ("⌘W", "Close the current window."),
                            ("⌘M", "Minimize the current window."),
                            ("⌃⌘F", "Enter or leave native Mac full screen."),
                            ("⌘Q", "Quit Textamp."),
                            (
                                "Reset View",
                                "Use View → Reset View to restore default pane proportions.",
                            ),
                            (
                                "Sidebar sections",
                                "Click a disclosure heading to hide or show its contents. Playlist names stay in the sidebar.",
                            ),
                        ],
                    },
                ];
                m
            }
            _ => return None,
        },
        Modal::Search => search(state),
        Modal::Input(dialog) => {
            let mut m = Surface::new(&format!("input:{}", dialog.title), state, &dialog.title);
            let describe = matches!(
                dialog.action_type,
                InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe)
            );
            if describe {
                m.title = "Describe Music".into();
                m.primary_disabled = dialog.input.as_str().trim().is_empty();
                m.examples = super::workspace::DESCRIPTION_EXAMPLES
                    .iter()
                    .map(|text| (*text).into())
                    .collect();
            }
            if matches!(dialog.action_type, InputDialogAction::CreatePlaylist { .. }) {
                m.primary_disabled = dialog.input.as_str().trim().is_empty();
                m.field("Playlist Name", dialog.input.as_str(), false);
            } else {
                m.field("", dialog.input.as_str(), false);
            }
            if let InputDialogAction::ImportPlaylist { generation } = &dialog.action_type {
                m.context_key = format!("import:{generation}");
                m.playlist_file = Some(PlaylistFile::Import {
                    generation: *generation,
                });
            }
            if let InputDialogAction::ExportPlaylist {
                generation,
                contents,
                format,
            } = &dialog.action_type
            {
                m.context_key = format!("export:{generation}:{}", format.extension());
                let path = std::path::PathBuf::from(dialog.input.as_str());
                m.playlist_file = Some(PlaylistFile::Export {
                    suggested_name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    action: QueueAction::ExportPlaylist {
                        generation: *generation,
                        contents: contents.clone(),
                        path,
                    }
                    .into(),
                });
            }
            m.primary_button(
                dialog.action_type.submit_label(),
                Operation::Key(KeyCode::Enter),
            );
            m
        }
        Modal::Lyrics(panel) => {
            let mut m = Surface::new("lyrics", state, format!("Lyrics · {}", panel.track.title));
            m.player_reader = true;
            m.layout = WindowLayout::Reading;
            let view = crate::app::lyrics::view(state).unwrap();
            m.loading = view.loading;
            m.toggles.push(Toggle {
                disabled: !view.synced,
                control: ChoiceControl::Checkbox,
                label: "Follow playback".into(),
                checked: view.follow,
                operation: Operation::Actions(vec![PlaybackAction::ToggleLyricsSync.into()]),
            });
            for (index, label) in view.languages.iter().enumerate() {
                m.tabs.push(Item {
                    glyph: None,
                    key: format!("language:{index}"),
                    label: label.clone(),
                    aliases: vec![],
                    selected: index == view.language,
                    operation: Operation::Actions(vec![
                        PlaybackAction::SelectLyricsLanguage(index).into(),
                    ]),
                });
            }
            m.button(
                "Reload Lyrics",
                Operation::Actions(vec![PlaybackAction::RetryLyrics.into()]),
            );
            if view.message.is_some() && !view.loading {
                m.button(
                    "Search Lyrics on the Web",
                    Operation::Actions(crate::app::lyrics::Control::Search.actions(state)),
                );
            }
            m.lyrics = Some(view);
            m
        }
        Modal::Text(popup) => {
            let mut m = Surface::new(
                &format!("text:{}:{}", popup.request_id, popup.title),
                state,
                &popup.title,
            );
            m.player_reader = popup.album_notes;
            m.loading = false;
            m.text = popup.text.clone();
            if let Some(query) = &popup.lyrics_query {
                m.button(
                    "Search Google",
                    Operation::Actions(vec![PlaybackAction::SearchLyrics(query.clone()).into()]),
                );
            }
            m
        }
        Modal::Confirmation { dialog, .. } => {
            let mut m = Surface::new(&format!("confirm:{}", dialog.title), state, &dialog.title);
            m.text = dialog.message.clone();
            m.button("Cancel", Operation::Confirm(false));
            m.primary_button("Confirm", Operation::Confirm(true));
            m
        }
        Modal::Sort(popup) => {
            let mut m = Surface::new(
                &format!(
                    "sort:{}:{}:{}",
                    state.browse_category.name(),
                    popup.column_title,
                    super::browser::columns(state)
                        .get(popup.column_idx)
                        .map(|column| column.identity.as_str())
                        .unwrap_or("")
                ),
                state,
                format!("View Options · {}", popup.column_title),
            );
            // View options are local controls, independent of library loading.
            m.loading = false;
            m.fixed_size = true;
            for choice in crate::app::view_options::choices(state, popup) {
                m.toggles.push(Toggle {
                    disabled: false,
                    control: match choice.kind {
                        crate::app::view_options::Kind::Radio => ChoiceControl::Radio,
                        crate::app::view_options::Kind::Checkbox => ChoiceControl::Checkbox,
                    },
                    label: choice.label.into(),
                    checked: choice.checked,
                    operation: Operation::ViewOption(choice.id),
                });
            }
            m
        }
        Modal::LibraryPicker => {
            let mut m = Surface::new("libraries", state, "Switch Library");
            for (index, choice) in crate::app::sources::library_choices(state)
                .iter()
                .enumerate()
            {
                m.row(
                    choice.artwork_key().unwrap_or_else(|| choice.label()),
                    match choice {
                        crate::app::sources::LibraryChoice::Navidrome {
                            source,
                            folder: None,
                            ..
                        } => format!("{} — All Folders", source.name),
                        crate::app::sources::LibraryChoice::Navidrome {
                            source,
                            folder: Some(_),
                            name,
                        } => format!("{} — Folder: {name}", source.name),
                        _ => choice.label(),
                    },
                    index == state.popups.library_picker_index,
                    index,
                );
            }
            m
        }
        Modal::ArtistBio(popup) => {
            let mut m = Surface::new(
                &format!("bio:{}", popup.artist_name),
                state,
                &popup.artist_name,
            );
            m.player_reader = true;
            m.layout = WindowLayout::Reading;
            m.loading = popup.task.is_some();
            m.text = if popup.task.is_some() {
                "Loading biography…".into()
            } else {
                popup.document.text.clone()
            };
            m.image = popup
                .document
                .images
                .get(popup.image_index)
                .map(|image| image.data.clone());
            if popup.document.images.len() > 1 {
                m.button("Previous Photo", Operation::Key(KeyCode::Left));
                m.button("Next Photo", Operation::Key(KeyCode::Right));
            }
            if popup.document.source_url.is_some() {
                m.button(
                    "Open Source",
                    Operation::Actions(vec![SearchAction::OpenBiographySource.into()]),
                );
            }
            m.bio_search = Some(state.web_search_provider);
            m
        }
        Modal::ArtistRadioPicker(picker) => {
            let mut m = Surface::new("artist-radio", state, "Artist Mix");
            m.layout = WindowLayout::Browser;
            m.multiple_choice = true;
            m.selected_names = picker
                .selected_artists
                .iter()
                .map(|artist| artist.title.clone())
                .collect();
            m.field("Search artists", &picker.query, false);
            m.text = format!(
                "Choose up to 12 artists. {} selected.",
                picker.selected_artists.len()
            );
            for (index, artist) in picker.filtered_artists.iter().enumerate() {
                m.row(
                    &artist.rating_key,
                    &artist.title,
                    picker
                        .selected_artists
                        .iter()
                        .any(|a| a.rating_key == artist.rating_key),
                    index,
                );
            }
            m.button("Cancel", Operation::Key(KeyCode::Esc));
            m.primary_button(
                "Start Radio",
                Operation::Actions(vec![SearchAction::ArtistRadioPickerLaunch.into()]),
            );
            m.primary_disabled = picker.selected_artists.is_empty();
            m
        }
        Modal::AdventureLauncher(launcher) => {
            let radio = launcher.purpose != TrackPickerPurpose::Journey;
            let mut m = Surface::new(
                if radio { "track-radio" } else { "journey" },
                state,
                if launcher.purpose == TrackPickerPurpose::StayCloseRadio {
                    "Stay Close Mix"
                } else if radio {
                    "Sonic Mix"
                } else {
                    "Journey"
                },
            );
            m.layout = WindowLayout::Browser;
            let available = crate::app::sources::navidrome::recommendations::available(state);
            m.text = if available { "" }
                else { "Journey needs sonic analysis. Enable sonic features and connect AudioMuse or a Navidrome sonic plugin in Settings → Libraries." }.into();
            m.loading = launcher.loading;
            let endpoints = launcher
                .start_track
                .as_ref()
                .zip(launcher.end_track.as_ref());
            let validation = endpoints.and_then(|(start, end)| {
                crate::services::sonic_adventure::validate_endpoints(start, end).err()
            });
            let count_valid = crate::app::journey::track_count(&launcher.track_count_input).is_ok();
            m.primary_disabled =
                !available || endpoints.is_none() || validation.is_some() || !count_valid;
            if let Some(error) = validation {
                m.text = error.to_string();
            }
            if !count_valid {
                m.text = crate::app::journey::InvalidTrackCount.to_string();
            }
            let summary = |track: &Option<crate::library::models::Track>| {
                track
                    .as_ref()
                    .map(|track| format!("{}\n{}", track.title, track.track_artist()))
                    .unwrap_or_else(|| "Choose a track".into())
            };
            {
                for (label, step, track) in [
                    (
                        crate::app::journey::START_TRACK,
                        AdventureStep::FindStartTrack,
                        &launcher.start_track,
                    ),
                    (
                        crate::app::journey::END_TRACK,
                        AdventureStep::FindEndTrack,
                        &launcher.end_track,
                    ),
                ]
                .into_iter()
                .filter(|(_, step, _)| !radio || *step == AdventureStep::FindStartTrack)
                {
                    let value = if launcher.step == step && !launcher.query.is_empty() {
                        launcher.query.clone()
                    } else {
                        track
                            .as_ref()
                            .map(|track| track.title.clone())
                            .unwrap_or_default()
                    };
                    m.field(label, &value, false);
                }
                if !radio {
                    m.field(
                        crate::app::journey::MAXIMUM_TRACKS,
                        &launcher.track_count_input,
                        false,
                    );
                }
            }
            for (label, step) in [
                (
                    crate::app::journey::START_TRACK,
                    AdventureStep::FindStartTrack,
                ),
                (crate::app::journey::END_TRACK, AdventureStep::FindEndTrack),
            ]
            .into_iter()
            .filter(|(_, step)| !radio || *step == AdventureStep::FindStartTrack)
            {
                m.endpoints.push(Item {
                    glyph: None,
                    key: label.into(),
                    label: summary(if step == AdventureStep::FindStartTrack {
                        &launcher.start_track
                    } else {
                        &launcher.end_track
                    }),
                    selected: launcher.step == step,
                    aliases: vec![],
                    operation: Operation::AdventureStep(step),
                });
            }
            match &launcher.stage {
                JourneyStage::Generating { .. } => {
                    m.form_hidden = true;
                    m.loading = true;
                    m.text =
                        "Finding a route between your tracks. Your current queue keeps playing."
                            .into();
                    m.button(
                        "Cancel",
                        Operation::Actions(vec![SearchAction::CloseAdventureLauncher.into()]),
                    );
                    return Some(m.finish());
                }
                JourneyStage::Ready(tracks) => {
                    m.form_hidden = true;
                    let duration = tracks.iter().filter_map(|track| track.duration).sum();
                    m.text = format!(
                        "{} tracks · {}. Play Journey replaces the current queue and starts playback.",
                        tracks.len(),
                        crate::util::format_duration(duration)
                    );
                    for (index, track) in tracks.iter().enumerate() {
                        m.rows.push(Item {
                            glyph: None,
                            key: track.rating_key.clone(),
                            label: format!(
                                "{}. {}\n{} · {}",
                                index + 1,
                                track.title,
                                track.track_artist(),
                                track.parent_title.as_deref().unwrap_or("")
                            ),
                            selected: index == launcher.item_index,
                            aliases: vec![],
                            operation: Operation::Actions(vec![]),
                        });
                    }
                    m.button(
                        "Change Tracks",
                        Operation::Actions(vec![SearchAction::AdventureLauncherEdit.into()]),
                    );
                    m.button(
                        "Cancel",
                        Operation::Actions(vec![SearchAction::CloseAdventureLauncher.into()]),
                    );
                    m.primary_button(
                        crate::app::journey::PLAY,
                        Operation::Actions(vec![SearchAction::AdventureLauncherPlay.into()]),
                    );
                    m.primary_disabled = false;
                    return Some(m.finish());
                }
                JourneyStage::Failed(error) => m.text = error.clone(),
                JourneyStage::Choosing => {}
            }
            if !matches!(launcher.drill, AdventureDrillLevel::Search) {
                m.button(
                    "Back to Results",
                    Operation::Actions(vec![SearchAction::AdventureLauncherBack.into()]),
                );
            }
            if !radio {
                m.button(crate::app::journey::SWAP, Operation::AdventureReverse);
            }
            m.button("Cancel", Operation::Key(KeyCode::Esc));
            if radio {
                m.text = if launcher.purpose == TrackPickerPurpose::StayCloseRadio {
                    "Choose a starting track. Stay close to its sound for continuous radio."
                } else {
                    "Choose a starting track. Explore outward as each new track guides the next."
                }
                .into();
                m.primary_button(
                    "Start Radio",
                    Operation::Actions(vec![SearchAction::StartPickedTrackRadio.into()]),
                );
                m.primary_disabled = !available || launcher.start_track.is_none();
            } else {
                m.primary_button(crate::app::journey::PREVIEW, Operation::AdventureGenerate);
            }
            let rows: Vec<_> = crate::app::journey::choices(launcher)
                .into_iter()
                .map(|choice| (choice.key().to_owned(), choice.title().to_owned()))
                .collect();
            for (index, (key, label)) in rows.into_iter().enumerate() {
                m.row(key, label, index == launcher.item_index, index);
            }
            m
        }
        Modal::Library(dialog) => library(state, dialog),
    };
    Some(m.finish())
}
fn search(state: &AppState) -> Surface {
    let mut m = Surface::new("search", state, "Search");
    m.layout = WindowLayout::Browser;
    m.field("Search", &state.search.query, false);
    tabs(&mut m, state.search.tab);
    if state.search.query.is_empty() {
        for (i, result) in state.search.recent.iter().enumerate() {
            m.row(
                format!("recent-{i}"),
                format!("{} · {}", result.kind_label(), result.label),
                i == state.list_state.search_item_index,
                i,
            );
        }
    } else if let Some(results) = &state.search.results {
        for (i, (key, label)) in result_rows(results, state.search.tab)
            .into_iter()
            .enumerate()
        {
            m.row(key, label, i == state.list_state.search_item_index, i);
        }
    }
    m.loading = state.library_loading
        || state.library.artists_loading
        || crate::app::handlers::dispatch_search::search_pending(state);
    if m.rows.is_empty() && !state.search.query.trim().is_empty() && !m.loading {
        m.text = format!("No results for “{}”", state.search.query);
    }
    m
}

fn library(state: &AppState, dialog: &LibraryDialog) -> Surface {
    let mut m = Surface::new("library-options", state, "Library");
    m.loading = false;
    match dialog {
        LibraryDialog::Library(options) => {
            m.library_inspector = true;
            m.id.push_str(&format!(":{:?}", options.choice.artwork_key()));
            m.title = options.choice.label();
            let key = options.choice.artwork_key();
            if options.page == crate::app::sources::options::OptionsPage::Cache {
                let metadata = state
                    .settings_state
                    .cache_entries
                    .iter()
                    .find(|(choice, _)| choice.artwork_key() == key)
                    .map(|(_, value)| match value {
                        Ok(bytes) => crate::util::format_bytes(*bytes),
                        Err(error) => error.clone(),
                    })
                    .unwrap_or_else(|| "Measuring…".into());
                let artwork = key
                    .as_ref()
                    .and_then(|key| state.settings_state.artwork_cache_entries.get(key))
                    .map(|(count, bytes)| {
                        format!("{count} images · {}", crate::util::format_bytes(*bytes))
                    })
                    .unwrap_or_else(|| "Measuring…".into());
                let mut details = vec![
                    format!("Metadata and analysis   {metadata}"),
                    format!("Downloaded artwork   {artwork}"),
                ];
                if matches!(
                    options.choice,
                    crate::app::sources::LibraryChoice::Navidrome { .. }
                ) {
                    let timestamp = key
                        .as_ref()
                        .and_then(|k| state.settings_state.catalog_sync_times.get(k))
                        .and_then(|t| {
                            time::OffsetDateTime::from_unix_timestamp((*t).try_into().ok()?).ok()
                        });
                    let date = timestamp
                        .map(|utc| {
                            let local = utc.to_offset(
                                time::UtcOffset::local_offset_at(utc)
                                    .unwrap_or(time::UtcOffset::UTC),
                            );
                            format!(
                                "{} at {:02}:{:02}",
                                local.date(),
                                local.hour(),
                                local.minute()
                            )
                        })
                        .unwrap_or_else(|| {
                            if state.settings_state.cache_task.is_some() {
                                "Checking…".into()
                            } else {
                                "No full sync recorded".into()
                            }
                        });
                    details.insert(0, format!("Last full Navidrome sync   {date}"));
                }
                if crate::app::sources::cache::running(&options.choice, state) {
                    details.push(crate::app::sources::cache::status(&options.choice, state));
                }
                m.text = details.join("\n");
            }
            use crate::app::sources::options::{OptionAction, OptionsPage};
            for (index, (action, label)) in options.items(state).into_iter().enumerate() {
                let op = Operation::Row(index);
                match action {
                    OptionAction::Close => {}
                    OptionAction::Rescan if options.page != OptionsPage::Cache => {}
                    OptionAction::Sonic => {
                        let enabled =
                            crate::app::sources::sonic::choice_key(&options.choice, state)
                                .is_some_and(|key| {
                                    !state.sources.sonic_disabled_libraries.contains(&key)
                                });
                        m.toggles.push(Toggle {
                            disabled: false,
                            control: ChoiceControl::Checkbox,
                            label: "Sonic / AI features".into(),
                            checked: enabled,
                            operation: op,
                        });
                    }
                    _ => m.button(label.replace("...", "…"), op),
                }
            }
            for page in options.pages(state) {
                m.tabs.push(Item {
                    glyph: None,
                    aliases: vec![],
                    key: page.name().into(),
                    label: page.name().into(),
                    selected: page == options.page,
                    operation: Operation::LibraryPage(page),
                });
            }
        }
        LibraryDialog::Local(form) => {
            m.id.push_str(":local");
            m.title = "Local Folder".into();
            for (i, label) in ["Folder path", "Name"].iter().enumerate() {
                m.field(*label, form.fields[i].value.as_str(), false);
            }
            m.text = form.error.clone().unwrap_or_default();
            m.primary_button("Save", Operation::Key(KeyCode::Enter));
        }
        LibraryDialog::Webdav(form) => {
            m.id.push_str(&format!(":webdav:{}", form.instance));
            m.title = "WebDAV".into();
            for (i, label) in ["Server URL", "Username", "Password", "Name"]
                .iter()
                .enumerate()
            {
                m.field(*label, form.fields[i].value.as_str(), i == 2);
            }
            m.text = form.error.clone().unwrap_or_default();
            m.primary_button("Save", Operation::Key(KeyCode::Enter));
        }
        LibraryDialog::Navidrome(form) => {
            m.id.push_str(&format!(":server:{}", form.source.id));
            m.title = "Subsonic / Navidrome".into();
            for (i, label) in [
                "Server URL",
                "Username",
                "Password",
                "Name",
                "AudioMuse URL",
                "AudioMuse username",
                "AudioMuse password",
                "Music server",
            ]
            .iter()
            .enumerate()
            {
                m.field(*label, form.fields[i].value.as_str(), i == 2 || i == 6);
            }
            m.text = form.error.clone().unwrap_or_default();
            m.primary_button("Save", Operation::Key(KeyCode::Enter));
        }
        LibraryDialog::AudioMuse(form) => {
            m.id.push_str(&format!(":analysis:{}", form.instance));
            m.title = "AudioMuse".into();
            for (i, label) in ["Server URL", "Username", "Password", "Music server"]
                .iter()
                .enumerate()
            {
                m.field(*label, form.fields[i].value.as_str(), i == 2);
            }
            m.text = form.error.clone().unwrap_or_default();
            m.primary_button("Save", Operation::Key(KeyCode::Enter));
        }
        LibraryDialog::Add { selected } => {
            m.id.push_str(":add");
            m.title = "Add Library".into();
            for (i, label) in ["Local Folder", "WebDAV", "Subsonic / Navidrome"]
                .iter()
                .enumerate()
            {
                m.row(*label, *label, i == *selected, i);
            }
        }
    }
    m
}

pub fn edit(state: &mut AppState, field: usize, value: String) -> Vec<Action> {
    if state.palette.open {
        return vec![];
    }
    if let Some(dialog) = state.popups.input_dialog_mut() {
        dialog.input = value.into();
        return vec![];
    }
    if let Some(picker) = state.popups.artist_radio_picker_mut() {
        picker.query = value;
        picker.focus = SearchFocus::Input;
        picker.item_index = 0;
        return vec![SearchAction::ArtistRadioPickerSearch.into()];
    }
    if let Some(launcher) = state.popups.adventure_launcher() {
        if launcher.purpose == TrackPickerPurpose::Journey {
            if field < 2 {
                let step = if field == 0 {
                    AdventureStep::FindStartTrack
                } else {
                    AdventureStep::FindEndTrack
                };
                let selected = if field == 0 {
                    launcher.start_track.as_ref()
                } else {
                    launcher.end_track.as_ref()
                };
                let current = if launcher.step == step && !launcher.query.is_empty() {
                    launcher.query.as_str()
                } else {
                    selected.map_or("", |track| track.title.as_str())
                };
                if value == current {
                    return vec![];
                }
                return vec![
                    SearchAction::AdventureLauncherSetEndpointQuery { step, query: value }.into(),
                ];
            }
            return vec![SearchAction::AdventureLauncherSetTrackCount(value).into()];
        }
        if field == 0 {
            return vec![SearchAction::AdventureLauncherSetQuery(value).into()];
        }
        return vec![SearchAction::AdventureLauncherSetTrackCount(value).into()];
    }
    if let Some(dialog) = state.popups.library_dialog_mut() {
        let fields: Option<&mut [_]> = match dialog {
            LibraryDialog::Local(f) => Some(&mut f.fields),
            LibraryDialog::Webdav(f) => Some(&mut f.fields),
            LibraryDialog::Navidrome(f) => Some(&mut f.fields[..]),
            LibraryDialog::AudioMuse(f) => Some(&mut f.fields),
            _ => None,
        };
        if let Some(fields) = fields
            && let Some(f) = fields.get_mut(field)
        {
            f.value = value.into();
            f.cursor = f.value.len();
        }
        return vec![];
    }
    if state.popups.search_active() || state.view == View::Search {
        return vec![SearchAction::SetSearchQuery(value).into()];
    }
    vec![]
}
/// Explicit top-level navigation discards an overlay's return destination.
/// Cancel through the shared handlers first so requests and draft owners finish.
pub fn navigate(
    command: PaletteCommandKind,
    state: &mut AppState,
    config: &crate::config::Config,
) -> Vec<Action> {
    if let PaletteCommandKind::ConfigureStation(key) = &command {
        return crate::app::radio_navigation::configure(state, key);
    }
    if matches!(command, PaletteCommandKind::ArtistRadio) {
        let mut actions = navigate(PaletteCommandKind::OpenRadioBrowser, state, config);
        actions.push(SearchAction::OpenArtistRadioPicker.into());
        return actions;
    }
    if matches!(command, PaletteCommandKind::SonicAdventureFromFocusedTrack) {
        let actions = crate::app::command_palette::run(command, state);
        state.desktop_library_root = LibraryRoot::Journey;
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.sources.nav_collection = None;
        state.desktop_player_visible = false;
        state.palette.close();
        return actions;
    }
    if matches!(command, PaletteCommandKind::SonicAdventure) {
        return super::workspace::open(LibraryRoot::Journey, state, config);
    }
    if matches!(
        command,
        PaletteCommandKind::Navidrome(
            crate::app::sources::navidrome::commands::Command::Collection(
                crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(
                    crate::audiomuse::Feature::Describe
                )
            )
        )
    ) {
        return super::workspace::open(LibraryRoot::DescribeMusic, state, config);
    }
    if super::radio::visible(state)
        && matches!(
            command,
            PaletteCommandKind::JumpToArtist(_)
                | PaletteCommandKind::JumpToAlbum(_)
                | PaletteCommandKind::GoToRandomAlbum
        )
    {
        state.desktop_library_root = LibraryRoot::Artists;
    }
    if matches!(command, PaletteCommandKind::OpenRadioBrowser) {
        if super::radio::visible(state) && project(state).is_none() {
            state.palette.close();
            state.desktop_player_visible = false;
            return vec![];
        }
        let mut actions = navigate(PaletteCommandKind::GotoLibrary, state, config);
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Library;
        state.sources.nav_collection = None;
        state.desktop_library_root = LibraryRoot::Radio;
        state.desktop_player_visible = false;
        if state.station_nav.columns.is_empty() && !state.station_nav.loading {
            actions.push(BrowseAction::LoadStations.into());
        }
        return actions;
    }

    if matches!(command, PaletteCommandKind::CloseColumn)
        && state.pane_track().is_none()
        && crate::app::sonic_details::visible(state).is_none()
    {
        let first = super::browser::first_content_column(state);
        if let Some(nav) = state.browse_nav_mut() {
            let last = nav.columns.len().saturating_sub(1);
            if first == 1 && last <= first {
                return vec![];
            }
            // Back closes the displayed leaf even while its parent retains focus.
            nav.focused_column = last;
        }
    }
    if matches!(
        command,
        PaletteCommandKind::GotoLibrary
            | PaletteCommandKind::OpenSearch
            | PaletteCommandKind::GotoGenres
            | PaletteCommandKind::GotoFolders
            | PaletteCommandKind::GotoQueue
            | PaletteCommandKind::GotoNowPlaying
    ) {
        if matches!(command, PaletteCommandKind::GotoLibrary) {
            state.desktop_library_root = LibraryRoot::Artists;
        }
        state.palette.close();
        state.previous_view = None;
        // Connection forms belong to the independent Settings window.
        let independent_form = state.popups.library_dialog().is_some();
        let mut actions = if !independent_form && project(state).is_some() {
            cancel(state, config)
        } else {
            vec![]
        };
        if !independent_form {
            state.popups.close_all();
        }
        actions.retain(|action| !matches!(action, Action::Navigation(_)));
        if matches!(command, PaletteCommandKind::GotoLibrary) {
            actions.push(NavigationAction::SetView(View::Browse).into());
        }
        actions.extend(crate::app::command_palette::run(command, state));
        actions
    } else {
        crate::app::command_palette::run(command, state)
    }
}

pub fn apply(
    operation: Operation,
    state: &mut AppState,
    config: &crate::config::Config,
) -> Vec<Action> {
    match operation {
        Operation::Native(_) => vec![],
        Operation::RadioSelect { column, key } => {
            let actions = super::radio::select(state, column, &key);
            if state
                .station_nav
                .columns
                .get(column + 1)
                .is_some_and(|child| child.key.as_deref() == Some(&key))
            {
                state.station_nav.focused_column = column + 1;
            }
            actions
        }
        Operation::Actions(actions) => actions,
        Operation::Palette(command) => {
            if crate::app::command_catalog::disabled_reason(state, &command).is_some() {
                return vec![];
            }
            if matches!(command, PaletteCommandKind::OpenCommandGroup(_)) {
                return crate::app::command_palette::run(command, state);
            }
            state.palette.close();
            navigate(command, state, config)
        }
        Operation::Confirm(yes) => key(
            if yes {
                KeyCode::Char('y')
            } else {
                KeyCode::Esc
            },
            state,
            config,
        ),
        Operation::Key(code) => {
            if super::radio::visible(state) && project(state).is_none() {
                if code == KeyCode::Esc {
                    return navigate(PaletteCommandKind::GotoLibrary, state, config);
                }
                return super::radio::key(state, code);
            }
            if code == KeyCode::Esc {
                return cancel(state, config);
            }
            // Library forms use their Save control, independent of text-field focus.
            if code == KeyCode::Enter
                && let Some(dialog) = state.popups.library_dialog_mut()
            {
                match dialog {
                    LibraryDialog::Local(f) => f.focus = 2,
                    LibraryDialog::Webdav(f) => f.focus = 4,
                    LibraryDialog::Navidrome(f) => f.focus = 8,
                    LibraryDialog::AudioMuse(f) => f.focus = 4,
                    _ => {}
                }
            }
            key(code, state, config)
        }
        Operation::LibraryPage(page) => {
            if let Some(LibraryDialog::Library(options)) = state.popups.library_dialog_mut() {
                options.set_page(page);
            }
            vec![]
        }
        Operation::AdventureStep(step) => vec![SearchAction::AdventureLauncherSetStep(step).into()],
        Operation::AdventureReverse => vec![SearchAction::AdventureLauncherReverse.into()],
        Operation::AdventureGenerate => vec![SearchAction::AdventureLauncherGenerate.into()],
        Operation::Tab(index) => {
            if let Some(tab) = SearchTab::all().get(index) {
                if let Some(l) = state.popups.adventure_launcher_mut() {
                    l.search_tab = *tab;
                    l.item_index = 0;
                } else {
                    return vec![SearchAction::SetSearchTab(*tab).into()];
                }
            }
            vec![]
        }
        Operation::ViewOption(option) => {
            if let Some(popup) = state.popups.sort_mut()
                && let Some(index) = popup
                    .options
                    .iter()
                    .position(|candidate| *candidate == option)
            {
                popup.selected_index = index;
                return crate::app::handlers::key_input::sort_popup::apply_selected_option(state);
            }
            vec![]
        }
        Operation::Row(index) => {
            if state.palette.open {
                return vec![];
            }
            if let Some(p) = state.popups.sort_mut() {
                p.selected_index = index;
                return crate::app::handlers::key_input::sort_popup::apply_selected_option(state);
            }
            if let Some(p) = state.popups.artist_radio_picker_mut() {
                p.item_index = index;
                p.focus = SearchFocus::Results;
                return vec![SearchAction::ArtistRadioPickerToggleArtist.into()];
            }
            if let Some(p) = state.popups.adventure_launcher_mut() {
                p.item_index = index;
                p.focus = SearchFocus::Results;
                return key(KeyCode::Enter, state, config);
            }
            if state.popups.library_picker_active() {
                state.popups.library_picker_index = index;
                return key(KeyCode::Enter, state, config);
            }
            if let Some(dialog) = state.popups.library_dialog_mut() {
                match dialog {
                    LibraryDialog::Library(p) => p.selected = index,
                    LibraryDialog::Add { selected } => *selected = index,
                    _ => {}
                }
                return key(KeyCode::Enter, state, config);
            }
            if state.popups.search_active() || state.view == View::Search {
                state.list_state.search_item_index = index;
                state.search.focus = SearchFocus::Results;

                return crate::app::library_details::search_result_actions(state);
            }
            vec![]
        }
    }
}
pub fn cancel(state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    if state.palette.open {
        state.palette.close();
        return vec![];
    }
    if state.popups.artist_radio_picker().is_some() {
        return vec![SearchAction::CloseArtistRadioPicker.into()];
    }
    if state.popups.adventure_launcher().is_some() {
        if state.desktop_library_root == LibraryRoot::Journey {
            state.desktop_library_root = LibraryRoot::Artists;
        }
        return vec![SearchAction::CloseAdventureLauncher.into()];
    }
    if state.desktop_library_root == LibraryRoot::DescribeMusic
        && state.popups.input_dialog().is_some_and(|dialog| {
            matches!(
                dialog.action_type,
                InputDialogAction::AudioMuseSearch(crate::audiomuse::Feature::Describe)
            )
        })
    {
        state.desktop_library_root = LibraryRoot::Artists;
    }
    key(KeyCode::Esc, state, config)
}
fn key(code: KeyCode, state: &mut AppState, config: &crate::config::Config) -> Vec<Action> {
    if (state.popups.search_active() || state.view == View::Search)
        && state.search.focus == SearchFocus::Results
        && matches!(code, KeyCode::Enter | KeyCode::Right)
    {
        return crate::app::library_details::search_result_actions(state);
    }
    crate::app::handlers::key_input::handle_key(
        KeyEvent::new(code, KeyModifiers::NONE),
        state,
        config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_new_playlist_desktop_form_accepts_a_name() {
        use crate::app::sources::navidrome::playlist_edit::Addition;
        let mut state = AppState::new();
        state.popups.set_input_dialog(Some(InputDialog {
            title: "New Playlist".into(),
            input: String::new().into(),
            action_type: InputDialogAction::CreatePlaylist {
                generation: state.library_generation,
                addition: Some(Addition::Track("song".into())),
            },
        }));
        let form = project(&state).unwrap();
        assert_eq!(form.primary.as_deref(), Some("Create"));
        assert!(form.primary_disabled);
        edit(&mut state, 0, "Weekend".into());
        assert!(!project(&state).unwrap().primary_disabled);
        let actions = apply(
            Operation::Key(KeyCode::Enter),
            &mut state,
            &Default::default(),
        );
        assert!(
            matches!(actions.as_slice(),[Action::Source(crate::app::sources::SourceAction::Navidrome(cmd))] if matches!(cmd.as_ref(),crate::app::sources::navidrome::NavAction::Command(crate::app::sources::navidrome::commands::Command::CreatePlaylist{name,..}) if name=="Weekend"))
        );
    }
    #[test]
    fn external_search_rows_disable_and_reenable_with_the_music_subject() {
        let mut state = AppState::new();
        crate::app::command_palette::open(&mut state);
        let empty = project(&state).unwrap();
        let index = empty
            .rows
            .iter()
            .position(|row| row.label == "Search Apple Music")
            .unwrap();
        assert!(empty.disabled_rows.contains_key(&index));
        assert!(
            apply(
                empty.rows[index].operation.clone(),
                &mut state,
                &Default::default()
            )
            .is_empty()
        );
        assert!(state.palette.open);
        state.queue.tracks = vec![crate::library::models::Track {
            rating_key: "playing".into(),
            title: "Spacewalk".into(),
            grandparent_title: Some("Momus".into()),
            ..Default::default()
        }];
        state.queue.index = Some(0);
        let playing = project(&state).unwrap();
        let index = playing
            .rows
            .iter()
            .position(|row| row.label == "Search Apple Music")
            .unwrap();
        assert!(!playing.disabled_rows.contains_key(&index));
        assert_ne!(empty.revision, playing.revision);
        let actions = apply(
            playing.rows[index].operation.clone(),
            &mut state,
            &Default::default(),
        );
        assert!(
            matches!(actions.as_slice(), [Action::System(SystemAction::OpenExternalSearch { query: Some(query), .. })] if query.text() == "Momus - Spacewalk")
        );
    }
    use crate::library::models::{Album, Artist, Genre, Playlist, SearchResults, Track};
    #[tokio::test]
    async fn missing_lyrics_finishes_without_inheriting_catalog_loading() {
        let mut state = AppState::new();
        state.library_loading = true;
        state.popups.set_text(Some(TextPopup {
            album_notes: false,
            lyrics_query: None,
            title: "Lyrics · Track".into(),
            text: "Loading…".into(),
            scroll: 0,
            request_id: 7,
        }));
        let (events, _inbox) = tokio::sync::mpsc::channel(8);
        crate::app::sources::navidrome::dispatch(
            crate::app::sources::navidrome::NavAction::Text {
                request_id: 7,
                result: Err("No lyrics available.".into()),
            },
            &mut state,
            &mut crate::audio::AudioPlayer::new_without_audio(),
            &mut crate::config::Config::default(),
            &events,
        )
        .await
        .unwrap();
        let model = project(&state).unwrap();
        assert_eq!(model.text, "No lyrics available.");
        assert!(!model.loading);
        assert!(state.library_loading);
    }

    #[tokio::test]
    async fn adventure_opens_with_unavailable_analysis_in_both_window_modes() {
        for multi in [false, true] {
            let mut state = AppState::new();
            state.desktop_frontend = true;
            let mut config = crate::config::Config::default();
            config.ui.desktop_separate_player = multi;
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            let (tx, _rx) = tokio::sync::mpsc::channel(16);
            crate::app::dispatch::dispatch_action(
                SearchAction::OpenAdventureLauncher.into(),
                &mut state,
                &mut audio,
                &mut config,
                &tx,
            )
            .await
            .unwrap();
            let model =
                project(&state).expect("opening must not silently disappear without analysis");
            assert_eq!(model.title, "Journey");
            assert!(model.tabs.is_empty());
            assert_eq!(model.endpoints.len(), 2);
            assert_eq!(model.primary.as_deref(), Some(crate::app::journey::PREVIEW));
            assert!(model.primary_disabled);
            assert!(model.text.contains("Settings"));
            assert!(state.queue.tracks.is_empty());
        }
    }
    #[tokio::test]
    async fn adventure_endpoint_selection_requires_explicit_generate_on_desktop() {
        let mut state = AppState::new();
        state.desktop_frontend = true;
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Fixture".into(),
            url: "http://127.0.0.1:9".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(
            crate::app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture-password".into(), None)
                    .unwrap(),
                source,
                extensions: std::collections::HashSet::from(["sonicSimilarity".into()]),
            },
        ));

        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        crate::app::dispatch::dispatch_action(
            SearchAction::OpenAdventureLauncher.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        for (key, artist) in [("start", "A"), ("end", "B")] {
            let launcher = state.popups.adventure_launcher_mut().unwrap();
            launcher.results = Some(SearchResults {
                tracks: vec![Track {
                    rating_key: key.into(),
                    title: key.into(),
                    grandparent_title: Some(artist.into()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            launcher.item_index = 0;
            crate::app::dispatch::dispatch_action(
                SearchAction::AdventureLauncherSelectTrack.into(),
                &mut state,
                &mut audio,
                &mut config,
                &tx,
            )
            .await
            .unwrap();
        }
        let model = project(&state).unwrap();
        assert_eq!(model.primary.as_deref(), Some(crate::app::journey::PREVIEW));
        assert_eq!(model.endpoints.len(), 2);
        assert!(matches!(
            state.popups.adventure_launcher().unwrap().stage,
            JourneyStage::Choosing
        ));
        assert!(state.queue.tracks.is_empty());
        assert!(
            state
                .popups
                .adventure_launcher()
                .unwrap()
                .end_track
                .is_some()
        );
        assert_eq!(
            model
                .fields
                .iter()
                .map(|field| field.label.as_str())
                .collect::<Vec<_>>(),
            [
                crate::app::journey::START_TRACK,
                crate::app::journey::END_TRACK,
                crate::app::journey::MAXIMUM_TRACKS
            ]
        );
        for (field, step) in [
            (0, AdventureStep::FindStartTrack),
            (1, AdventureStep::FindEndTrack),
        ] {
            let query = format!("endpoint {field}");
            for action in edit(&mut state, field, query.clone()) {
                crate::app::dispatch::dispatch_action(
                    action,
                    &mut state,
                    &mut audio,
                    &mut config,
                    &tx,
                )
                .await
                .unwrap();
            }
            let launcher = state.popups.adventure_launcher().unwrap();
            assert_eq!(launcher.step, step);
            assert_eq!(launcher.query, query);
            assert!(state.queue.tracks.is_empty());
        }
    }

    #[tokio::test]
    async fn player_tools_are_available_and_run_from_library_and_fullscreen() {
        for fullscreen in [false, true] {
            let mut state = AppState::new();
            state.fullscreen = if fullscreen {
                crate::app::state::FullscreenView::Visualizer
            } else {
                crate::app::state::FullscreenView::Closed
            };
            state.queue.tracks = ["one", "two", "three"]
                .map(|key| Track {
                    rating_key: key.into(),
                    title: key.into(),
                    ..Default::default()
                })
                .into();
            state.queue.index = Some(0);
            let mut audio = crate::audio::AudioPlayer::new_without_audio();
            let mut config = crate::config::Config::default();
            let (tx, _rx) = tokio::sync::mpsc::channel(16);
            for shuffled in [true, false] {
                state.palette.open = true;
                let model = project(&state).unwrap();
                let operation = model
                    .rows
                    .iter()
                    .find(|r| {
                        matches!(
                            r.operation,
                            Operation::Palette(PaletteCommandKind::QueueTool(
                                crate::app::presentation::NpSidebarButton::Shuffle
                            ))
                        )
                    })
                    .expect("available queue edit")
                    .operation
                    .clone();
                for action in apply(operation, &mut state, &config) {
                    crate::app::dispatch::dispatch_action(
                        action,
                        &mut state,
                        &mut audio,
                        &mut config,
                        &tx,
                    )
                    .await
                    .unwrap();
                }
                assert_eq!(
                    state.queue.sort_mode == crate::app::state::QueueSortMode::Shuffle,
                    shuffled
                );
            }
            assert_eq!(
                state
                    .queue
                    .tracks
                    .iter()
                    .map(|t| t.rating_key.as_str())
                    .collect::<Vec<_>>(),
                ["one", "two", "three"]
            );
            assert_eq!(state.current_track().unwrap().rating_key, "one");
        }
    }

    #[tokio::test]
    async fn playlist_cover_and_grouping_toggles_remain_visible_and_update_shared_state() {
        let mut state = AppState::new();
        state.browse_category = BrowseCategory::Playlists;
        let tracks: Vec<_> = [("one", "Album One"), ("two", "Album Two")]
            .into_iter()
            .map(|(key, album)| Track {
                rating_key: key.into(),
                title: key.into(),
                parent_rating_key: Some(album.into()),
                parent_title: Some(album.into()),
                thumb: Some(album.into()),
                ..Default::default()
            })
            .collect();
        state.playlist_nav.columns = vec![
            BrowseColumn::new("playlists", vec![]),
            BrowseColumn::new_with_tracks("Fixture", BrowseItem::from_tracks(&tracks), tracks),
        ];
        state.playlist_nav.focused_column = 1;
        state.playlist_nav.columns[1].lazy = Some(LazyPlaylist {
            key: "playlist".into(),
            total: Some(2),
            loading: false,
        });
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        crate::app::dispatch::dispatch_action(
            SearchAction::OpenSortPopup.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        let initial = project(&state).unwrap();
        for label in ["Show artwork", "Group by album"] {
            assert!(
                !initial
                    .toggles
                    .iter()
                    .find(|t| t.label == label)
                    .unwrap()
                    .checked
            );
        }
        assert!(
            initial
                .toggles
                .iter()
                .find(|t| t.label == "Playlist order")
                .unwrap()
                .checked
        );
        for label in [
            "Show artwork",
            "Group by album",
            "Group by album",
            "Show artwork",
        ] {
            let before = project(&state).unwrap();
            let toggle = before.toggles.iter().find(|t| t.label == label).unwrap();
            let expected = !toggle.checked;
            let actions = apply(toggle.operation.clone(), &mut state, &config);
            for action in actions {
                crate::app::dispatch::dispatch_action(
                    action,
                    &mut state,
                    &mut audio,
                    &mut config,
                    &tx,
                )
                .await
                .unwrap();
            }
            let after = project(&state).unwrap();
            assert_eq!(
                after
                    .toggles
                    .iter()
                    .find(|t| t.label == label)
                    .unwrap()
                    .checked,
                expected
            );
            assert_ne!(before.revision, after.revision);
            if state.playlist_nav.columns[1].grouped_by_album {
                assert!(
                    state.playlist_nav.columns[1]
                        .items
                        .iter()
                        .all(|item| matches!(item, BrowseItem::Album { thumb: Some(_), .. }))
                );
            }
        }
        assert!(state.queue.tracks.is_empty());
        assert_eq!(state.playback.status, crate::app::PlayStatus::Stopped);
    }

    #[tokio::test]
    async fn album_view_options_offer_grid_and_list_without_playlist_grouping() {
        let mut state = AppState::new();
        state.library_loading = true;
        state.artist_nav.columns = vec![BrowseColumn::new(
            "albums",
            BrowseItem::from_albums(
                &[Album {
                    rating_key: "album".into(),
                    title: "Album".into(),
                    ..Default::default()
                }],
                &Default::default(),
            ),
        )];
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        crate::app::dispatch::dispatch_action(
            SearchAction::OpenSortPopup.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        let model = project(&state).unwrap();
        assert!(
            !model.loading,
            "View Options must not inherit library loading"
        );
        for layout in ["Grid", "List"] {
            assert!(model.toggles.iter().any(|toggle| toggle.label == layout));
        }
        assert!(
            !model
                .toggles
                .iter()
                .any(|toggle| toggle.label == "Group by album")
        );
    }

    #[tokio::test]
    async fn all_tracks_destination_exposes_all_shared_sort_modes() {
        let mut state = AppState::new();
        state.desktop_library_root = LibraryRoot::Tracks;
        let tracks = vec![Track {
            rating_key: "track".into(),
            title: "Track".into(),
            ..Default::default()
        }];
        state.artist_nav.columns = vec![
            BrowseColumn::new("artists", vec![BrowseItem::AllArtists]),
            BrowseColumn::new_with_tracks("all tracks", BrowseItem::from_tracks(&tracks), tracks),
        ];
        state.artist_nav.focused_column = 1;
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        crate::app::dispatch::dispatch_action(
            SearchAction::OpenSortPopup.into(),
            &mut state,
            &mut audio,
            &mut config,
            &tx,
        )
        .await
        .unwrap();
        let surface = project(&state).unwrap();
        for label in [
            "Library order",
            "By artist",
            "By album",
            "By title",
            "By duration",
            "Shuffled",
            "Reverse order",
        ] {
            assert!(
                surface.toggles.iter().any(|r| r.label == label),
                "missing {label}"
            );
        }
        assert!(super::super::browser::columns(&state)[0].hidden);
        assert!(!super::super::browser::columns(&state)[1].hidden);
    }

    #[tokio::test]
    async fn library_navigation_dismisses_related_without_restoring_now_playing() {
        let mut state = AppState::new();
        state.view = View::Related;
        state.previous_view = Some(View::NowPlaying);
        let mut config = crate::config::Config::default();
        let mut audio = crate::audio::AudioPlayer::new_without_audio();
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        for action in navigate(PaletteCommandKind::GotoLibrary, &mut state, &config) {
            crate::app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert_eq!(state.view, View::Browse);
        assert_eq!(state.browse_category, BrowseCategory::Library);
        assert_eq!(state.previous_view, None);
        assert!(project(&state).is_none());
        for action in cancel(&mut state, &config) {
            crate::app::dispatch::dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert_eq!(state.view, View::Browse);
        assert!(state.queue.tracks.is_empty());
    }

    #[test]
    fn global_search_rows_use_the_shared_selection_order() {
        let results = SearchResults {
            global_order: vec![1, 4, 0, 2, 3],
            artists: vec![Artist {
                rating_key: "artist".into(),
                title: "Artist".into(),
                ..Default::default()
            }],
            albums: vec![Album {
                rating_key: "album".into(),
                title: "Album".into(),
                ..Default::default()
            }],
            playlists: vec![Playlist {
                rating_key: "playlist".into(),
                title: "Playlist".into(),
                key: "playlist".into(),
                playlist_type: "audio".into(),
                composite: None,
                duration: None,
                leaf_count: None,
                added_at: None,
                updated_at: None,
                smart: false,
            }],
            genres: vec![Genre {
                key: "genre".into(),
                title: "Genre".into(),
                count: None,
            }],
            tracks: vec![Track {
                rating_key: "track".into(),
                title: "Track".into(),
                ..Default::default()
            }],
        };
        let rows = result_rows(&results, SearchTab::Global);
        for (index, (key, label)) in rows.iter().enumerate() {
            let (tab, local) =
                crate::app::handlers::dispatch_search::resolve_global_index(&results, index);
            assert_eq!(*key, result_rows(&results, tab)[local].0);
            assert!(
                label
                    .lines()
                    .any(|line| line.starts_with(&format!("{} · ", tab.result_kind())))
            );
        }
    }
    #[test]
    fn search_result_click_and_result_enter_navigate_to_library_details() {
        let mut state = AppState::new();
        state.popups.set_search_active(true);
        state.search.tab = SearchTab::Tracks;
        state.search.focus = SearchFocus::Results;
        state.search.results = Some(SearchResults {
            tracks: vec![Track {
                rating_key: "track".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        for operation in [Operation::Row(0), Operation::Key(KeyCode::Enter)] {
            let actions = apply(operation, &mut state, &Default::default());
            assert!(matches!(
                actions.as_slice(),
                [
                    Action::Settings(SettingsAction::RememberSearchResult(_)),
                    Action::Browse(BrowseAction::OpenDetailsInLibrary {
                        destination: crate::app::library_details::Destination::Track,
                        ..
                    })
                ]
            ));
        }
    }
    #[test]
    fn native_query_edits_and_tab_selection_dispatch_shared_search_actions() {
        let mut state = AppState::new();
        state.popups.set_search_active(true);
        let actions = edit(&mut state, 0, "Album".into());
        assert!(
            matches!(actions.as_slice(),[Action::Search(SearchAction::SetSearchQuery(q))] if q=="Album")
        );
        let index = SearchTab::all()
            .iter()
            .position(|t| *t == SearchTab::Tracks)
            .unwrap();
        let actions = apply(Operation::Tab(index), &mut state, &Default::default());
        assert!(matches!(
            actions.as_slice(),
            [Action::Search(SearchAction::SetSearchTab(
                SearchTab::Tracks
            ))]
        ));
    }
    #[test]
    fn date_added_sort_uses_radio_choices_and_reverse_order_is_a_checkbox() {
        let mut state = AppState::new();
        let mut column = BrowseColumn::new(
            "Recently added",
            vec![BrowseItem::Album {
                key: "album".into(),
                title: "Album".into(),
                artist: "Artist".into(),
                year: None,
                thumb: None,
                is_placeholder: false,
            }],
        );
        column.date_added_order = Some(vec!["album".into()]);
        column.sort_mode = ColumnSortMode::ByDateAdded;
        column.sort_ascending = false;
        state.artist_nav.columns = vec![column];
        let (tx, _) = tokio::sync::mpsc::channel(8);
        crate::app::handlers::dispatch_search::dispatch(
            &tx,
            SearchAction::OpenSortPopup,
            &mut state,
        )
        .unwrap();
        let model = project(&state).unwrap();
        assert!(model.toggles.iter().any(|t| t.label == "Date added"
            && t.checked
            && matches!(t.control, ChoiceControl::Radio)));
        assert!(model.toggles.iter().any(|t| t.label == "Reverse order"
            && !t.checked
            && matches!(t.control, ChoiceControl::Checkbox)));
        assert!(
            model
                .toggles
                .iter()
                .any(|t| t.label == "Show artwork" && matches!(t.control, ChoiceControl::Checkbox))
        );
    }
    #[test]
    fn escape_closes_palette_even_with_a_query() {
        let mut state = AppState::new();
        crate::app::command_palette::open_with_query(&mut state, "Radio");
        apply(
            Operation::Key(KeyCode::Esc),
            &mut state,
            &Default::default(),
        );
        assert!(!state.palette.open);
    }
    #[test]
    fn close_column_goes_back_from_playlist_child_while_parent_keeps_focus() {
        let mut state = AppState::new();
        state.category_column_focused = false;
        state.view = View::Browse;
        state.browse_category = BrowseCategory::Playlists;
        state.playlist_nav.columns = vec![
            BrowseColumn::new("playlists", vec![]),
            BrowseColumn::new("playlist albums", vec![]),
            BrowseColumn::new("album tracks", vec![]),
        ];
        state.playlist_nav.focused_column = 1;
        navigate(
            PaletteCommandKind::CloseColumn,
            &mut state,
            &crate::config::Config::default(),
        );
        assert_eq!(state.playlist_nav.columns.len(), 2);
        assert_eq!(state.playlist_nav.focused_column, 1);
    }
    #[test]
    fn export_file_request_is_independent_of_playback_advancing() {
        let mut state = AppState::new();
        state.view = View::NowPlaying;
        state.queue.tracks = vec![
            Track {
                rating_key: "first".into(),
                ..Default::default()
            },
            Track {
                rating_key: "second".into(),
                ..Default::default()
            },
        ];
        state.queue.index = Some(0);
        state.popups.set_input_dialog(Some(InputDialog {
            title: "Export Playlist".into(),
            input: "Queue.csv".into(),
            action_type: InputDialogAction::ExportPlaylist {
                generation: state.library_generation,
                contents: std::sync::Arc::new(b"Title,Artist".to_vec()),
                format: crate::services::playlist_export::Format::Csv,
            },
        }));
        let before = project(&state).unwrap();
        state.queue.index = Some(1);
        let after = project(&state).unwrap();
        assert_eq!(before.id, after.id);
        assert_eq!(before.revision, after.revision);
        assert!(after.playlist_file.is_some());
    }
}

/// A sidebar heading's existing destinations presented as a library column.
pub(super) fn sidebar_group(state: &AppState, title: &str, selected: usize) -> Option<Surface> {
    let rows = super::catalog::sidebar(state);
    if !rows.iter().any(|row| row.header && row.label == title) {
        return None;
    }
    let mut model = Surface::new(&format!("sidebar-group:{title}"), state, title);
    model.layout = WindowLayout::Browser;
    model.loading = false;
    let query = if title == "Playlists" {
        let value = state.playlist_group_filter.as_deref().unwrap_or_default();
        model.field("Filter Playlists", value, false);
        value
    } else {
        ""
    };
    for (ordinal, (index, row)) in crate::app::sidebar::filtered_group_rows(state, title, query)
        .into_iter()
        .enumerate()
    {
        model.rows.push(Item {
            glyph: super::sidebar::glyph(&row),
            key: row.playlist_key.unwrap_or_else(|| row.label.clone()),
            label: row.label,
            aliases: vec![],
            selected: ordinal == selected,
            operation: Operation::Row(index),
        });
    }
    Some(model.finish())
}

#[cfg(test)]
mod sidebar_group_tests {
    use super::*;
    #[test]
    fn option_opened_heading_uses_existing_sidebar_destinations() {
        let state = AppState::new();
        let model = sidebar_group(&state, "Albums", 0).unwrap();
        assert_eq!(model.rows[0].label, "All");
        let Operation::Row(index) = model.rows[0].operation else {
            panic!("Sidebar destination expected");
        };
        let rows = super::super::catalog::sidebar(&state);
        assert_eq!(rows[index].library_root, Some(LibraryRoot::Albums));
        assert!(model.rows[0].selected);
        for item in &model.rows {
            let Operation::Row(index) = item.operation else {
                panic!("Expected destination")
            };
            assert!(item.glyph.is_some());
            assert_eq!(item.glyph, super::super::sidebar::glyph(&rows[index]));
        }
    }
}
