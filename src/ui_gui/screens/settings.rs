//! Settings — rendered as tabbed content inside the Settings popup
//! (the GUI wraps this in `popups::settings_popup`).
//!
//! Mirrors the TUI's sections so the feature set stays in sync. The
//! GUI hides the "About" tab because a dedicated File > About popup
//! already covers that content; if the shared state lands on `About`
//! (e.g. via keyboard nav), we fall through to the Account section.

use iced::widget::{button, column, container, row as iced_row, scrollable, text, Column, Row, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use crate::app::action::{Action, SettingsAction};
use crate::app::state::{AppState, ConnectionState, RefreshCategory, SettingsSection};
use crate::app::theme::ThemeName;
use crate::config::settings::{UI_SCALE_MAX, UI_SCALE_MIN, UI_SCALE_STEP};
use crate::ui_gui::message::GuiMessage;
use crate::ui_gui::widgets::transport_bar::popout_button_style;

pub fn view<'a>(state: &'a AppState, ui_scale: f32) -> Element<'a, GuiMessage> {
    // GUI exposes Account + Textamp + Cache; About lives in the File
    // menu popup. If the shared state has drifted to About, render
    // Account.
    let current = match state.settings_state.section {
        SettingsSection::About => SettingsSection::Account,
        other => other,
    };
    let tabs = iced_row![
        tab_button("Account", current == SettingsSection::Account, SettingsSection::Account),
        tab_button("Textamp", current == SettingsSection::Textamp, SettingsSection::Textamp),
        tab_button("Cache",   current == SettingsSection::Cache,   SettingsSection::Cache),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let body: Element<'a, GuiMessage> = match current {
        SettingsSection::Account => account_section(state),
        SettingsSection::Textamp => textamp_section(state, ui_scale),
        SettingsSection::Cache   => cache_section(state),
        SettingsSection::About   => account_section(state),
    };

    // Shrink height so the wrapping scrollable's content_h matches
    // the actual content size — without this the column stretches
    // to fill the full scrollable area and the scroll bar lets the
    // user scroll past the last row into empty space.
    container(column![tabs, body].spacing(14).padding([4, 0]))
        .width(Length::Fill)
        .height(Length::Shrink)
        .into()
}

fn tab_button(label: &'static str, active: bool, section: SettingsSection) -> Element<'static, GuiMessage> {
    button(text(label).size(15))
        .padding([4, 14])
        .on_press(GuiMessage::SetSettingsSection(section))
        .style(move |theme: &Theme, status: button::Status| {
            let p = theme.extended_palette();
            let (bg, fg) = if active {
                (p.primary.strong.color, p.primary.strong.text)
            } else {
                match status {
                    button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                    _ => (p.background.weak.color, p.background.base.text),
                }
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border { color: p.background.strong.color, width: 1.0, radius: 4.0.into() },
                ..button::Style::default()
            }
        })
        .into()
}

fn account_section(state: &AppState) -> Element<'_, GuiMessage> {
    let connection_line = match &state.connection {
        ConnectionState::Connected { username, has_plex_pass } => {
            let pass = if *has_plex_pass { "Plex Pass" } else { "free account" };
            format!("Signed in as {username} - {pass}")
        }
        ConnectionState::Authenticating => "Signing in...".to_string(),
        ConnectionState::Connecting => "Connecting to server...".to_string(),
        ConnectionState::AuthPending { pin_code, .. } => format!("plex.tv/link PIN: {pin_code}"),
        ConnectionState::Disconnected => "Not signed in.".to_string(),
        ConnectionState::Error(msg) => format!("Connection error: {msg}"),
    };

    let server_line = match (&state.connected_server_url, state.available_servers.iter().find(|s| {
        state.connected_server_url.as_ref().map_or(false, |url| s.connections.iter().any(|c| &c.uri == url))
    })) {
        (Some(_), Some(server)) => format!("Server: {}", server.name),
        (Some(url), None) => format!("Server: {url}"),
        _ => "Server: (not connected)".to_string(),
    };

    let mut libs: Vec<Element<'_, GuiMessage>> = Vec::new();
    libs.push(text(format!("Music libraries ({}):", state.libraries.len())).size(15).into());
    for lib in &state.libraries {
        let is_active = state.active_library.as_ref() == Some(&lib.key);
        let label = format!("{}{}", if is_active { "* " } else { "  " }, lib.title);
        let key = lib.key.clone();
        libs.push(
            button(text(label).size(14))
                .width(Length::Fill)
                .padding([3, 10])
                .on_press(GuiMessage::Action(Action::Settings(SettingsAction::SelectLibrary(key))))
                .style(move |theme: &Theme, status: button::Status| {
                    let p = theme.extended_palette();
                    // Use the high-contrast primary pair for the active
                    // row. The earlier `(primary.weak, primary.strong)`
                    // combo painted blue text on a blue background in
                    // every theme — unreadable, especially Dark.
                    let (bg, fg) = if is_active {
                        (p.primary.strong.color, p.primary.strong.text)
                    } else {
                        match status {
                            button::Status::Hovered => (p.background.weak.color, p.background.weak.text),
                            _ => (Color::TRANSPARENT, p.background.base.text),
                        }
                    };
                    button::Style {
                        background: Some(Background::Color(bg)),
                        text_color: fg,
                        border: Border::default(),
                        ..button::Style::default()
                    }
                })
                .into(),
        );
    }
    // Cap the libraries list at a small scrollable so the rest of the
    // section never gets pushed off-screen on accounts with many libs.
    let libs_col: Element<'_, GuiMessage> =
        scrollable(Column::with_children(libs).spacing(2))
            .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
            .style(crate::ui_gui::widgets::chunky_scrollable_style)
            .height(Length::Fixed(96.0))
            .into();

    let sign_out = action_btn("Sign out", Action::Settings(SettingsAction::Logout));

    let content = column![
        text(connection_line).size(15),
        text(server_line).size(14),
        Space::with_height(Length::Fixed(4.0)),
        libs_col,
        Space::with_height(Length::Fixed(8.0)),
        sign_out,
    ]
    .spacing(4);

    container(content).width(Length::Fill).into()
}

/// Cache tab — library + artwork + subfolder cache info, the clear /
/// crawl buttons, and a brief explanation that subfolder caching has
/// to be done client-side.
fn cache_section(state: &AppState) -> Element<'_, GuiMessage> {
    let cache_info = cache_info_block(state);

    let (crawl_label, crawl_action) = if state.subfolder_preload_active {
        ("Stop subfolder crawl", SettingsAction::StopSubfolderCrawl)
    } else {
        ("Start subfolder crawl", SettingsAction::StartSubfolderCrawl)
    };

    // Two rows so no button has to wrap onto a second line. Top row:
    // refresh + clears. Bottom row: subfolder controls (clear and
    // start/stop crawl), which are tied to the folder feature.
    let mk_btn = |label: &str, msg: GuiMessage| -> Element<'_, GuiMessage> {
        button(text(label.to_string()).size(14))
            .padding([4, 12])
            .on_press(msg)
            .style(popout_button_style)
            .into()
    };
    let cache_row1 = iced_row![
        mk_btn("Refresh all cache",   GuiMessage::Action(Action::Settings(SettingsAction::RefreshAllCache))),
        mk_btn("Clear library cache", GuiMessage::Action(Action::Settings(SettingsAction::ClearLibraryCache))),
        mk_btn("Clear artwork cache", GuiMessage::Action(Action::Settings(SettingsAction::ClearArtworkCache))),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let cache_row2 = iced_row![
        mk_btn("Clear subfolder cache", GuiMessage::Action(Action::Settings(SettingsAction::ClearSubfolderCache))),
        mk_btn(crawl_label,             GuiMessage::Action(Action::Settings(crawl_action))),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let cache_buttons = column![cache_row1, cache_row2].spacing(6);

    let folder_help = text(
        "Plex does not provide folders via the API the same way as it does \
         the Library. A manual crawl is necessary."
    ).size(14);

    let content = column![
        cache_info,
        Space::with_height(Length::Fixed(4.0)),
        cache_buttons,
        Space::with_height(Length::Fixed(8.0)),
        folder_help,
    ]
    .spacing(4);

    container(content).width(Length::Fill).into()
}

/// Cache counts + per-row age and on-disk size, rendered as a
/// 4-column table (Category | Count | Age | Size). Sizes for library
/// categories come from `state.library_cache_stats.breakdown`; the
/// Artwork row uses `state.artwork.cache_stats`; the Folders row uses
/// the breakdown's "folders" field plus the in-memory listing count.
fn cache_info_block(state: &AppState) -> Element<'_, GuiMessage> {
    let connected = matches!(state.connection, ConnectionState::Connected { .. });
    if !connected || state.active_library.is_none() {
        return text("Cache: (signed out)").size(14).into();
    }

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Library breakdown is keyed by field name ("tracks", "albums",
    // "artists", "playlist tracks", "folders", "compilations",
    // "genres", "stations"). We look these up by name to fill the
    // Size column. Field names are case-sensitive.
    let library_size = |field: &str| -> Option<u64> {
        state
            .library_cache_stats
            .as_ref()
            .and_then(|(_, breakdown)| breakdown.iter().find(|(k, _)| k == field).map(|(_, v)| *v))
    };

    let alias_count: usize = state.library.artist_aliases.values().map(|s| s.len()).sum();
    let row_specs: &[(&str, usize, bool, Option<RefreshCategory>, Option<&str>)] = &[
        ("Artists",   state.library.artists.len(),    false, Some(RefreshCategory::Artists),   Some("artists")),
        ("Albums",    state.library.albums.len(),     false, Some(RefreshCategory::Albums),    Some("albums")),
        ("Tracks",    state.library.all_tracks.len(), state.library.all_tracks.is_empty(), Some(RefreshCategory::AllTracks), Some("tracks")),
        ("Playlists", state.library.playlists.len(),  false, Some(RefreshCategory::Playlists), Some("playlist tracks")),
        ("Genres",    state.library.genres.len(),     false, Some(RefreshCategory::Genres),    Some("genres")),
        ("Moods",     state.library.moods.len(),      false, Some(RefreshCategory::Moods),     None),
        ("Styles",    state.library.styles.len(),     false, Some(RefreshCategory::Styles),    None),
        ("Stations",  state.stations.len(),           false, Some(RefreshCategory::Stations),  Some("stations")),
        ("Aliases",   alias_count,                    false, None,                              None),
    ];

    let header_cell = |s: &str, w: f32, accent: bool| -> Element<'_, GuiMessage> {
        let t = text(s.to_string()).size(13);
        let styled: Element<'_, GuiMessage> = if accent {
            container(t)
                .padding([2, 6])
                .width(Length::Fixed(w))
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(Background::Color(p.background.weak.color)),
                        text_color: Some(p.background.weak.text),
                        ..container::Style::default()
                    }
                })
                .into()
        } else {
            container(t).padding([2, 6]).width(Length::Fixed(w)).into()
        };
        styled
    };

    let col_w_label = 110.0;
    let col_w_count =  90.0;
    let col_w_age   = 110.0;
    let col_w_size  =  90.0;

    let header = Row::new()
        .push(header_cell("Category", col_w_label, true))
        .push(header_cell("Count",    col_w_count, true))
        .push(header_cell("Age",      col_w_age,   true))
        .push(header_cell("Size",     col_w_size,  true));

    let mut rows_col = Column::new().push(header).spacing(0);

    let mk_row = |label: String, count_str: String, age_str: String, size_str: String| -> Row<'_, GuiMessage> {
        Row::new()
            .push(container(text(label).size(13))
                .padding([1, 6]).width(Length::Fixed(col_w_label)))
            .push(container(text(count_str).size(13))
                .padding([1, 6]).width(Length::Fixed(col_w_count)))
            .push(container(text(age_str).size(13))
                .padding([1, 6]).width(Length::Fixed(col_w_age)))
            .push(container(text(size_str).size(13))
                .padding([1, 6]).width(Length::Fixed(col_w_size)))
    };

    for (label, count, is_loading, cat, breakdown_field) in row_specs {
        if *count == 0 && !is_loading { continue; }
        let count_str = if *is_loading { "loading".to_string() } else { format_count(*count) };
        let age_str = cat
            .and_then(|c| state.cache_mgmt.category_timestamps.get(&c))
            .map(|&ts| format_duration(now_ts.saturating_sub(ts)))
            .unwrap_or_default();
        let refreshing = cat.map_or(false, |c| state.cache_mgmt.background_refresh.contains(&c));
        let age_str = if refreshing { format!("{age_str} (refreshing)") } else { age_str };
        let size_str = breakdown_field
            .and_then(|f| library_size(f))
            .map(format_bytes)
            .unwrap_or_else(|| "\u{2014}".to_string());
        rows_col = rows_col.push(mk_row(label.to_string(), count_str, age_str, size_str));
    }

    // Folders row — uses the in-memory count of cached listings + the
    // breakdown's "folders" size if present.
    let root_folder_count = state.folder_state.as_ref()
        .and_then(|fs| fs.columns.first())
        .map(|col| col.items.iter()
            .filter(|item| item.item_type == crate::services::FolderItemType::Folder)
            .count())
        .unwrap_or(0);
    let cached_listings = state.folder_contents_cache.len();
    if root_folder_count > 0 || cached_listings > 0 {
        let count_str = if cached_listings > 0 {
            format!("{}/{}", cached_listings, root_folder_count)
        } else {
            format!("{}", root_folder_count)
        };
        let age_str = if state.subfolder_preload_active {
            "crawling".to_string()
        } else {
            String::new()
        };
        let size_str = library_size("folders")
            .map(format_bytes)
            .unwrap_or_else(|| "\u{2014}".to_string());
        rows_col = rows_col.push(mk_row("Folders".to_string(), count_str, age_str, size_str));
    }

    // Artwork row — count + total disk bytes from artwork.cache_stats.
    if let Some((art_count, art_bytes)) = state.artwork.cache_stats {
        if art_count > 0 || art_bytes > 0 {
            rows_col = rows_col.push(mk_row(
                "Artwork".to_string(),
                format_count(art_count),
                String::new(),
                format_bytes(art_bytes),
            ));
        }
    }

    // Waveforms row — same shape as Artwork.
    if let Some((wf_count, wf_bytes)) = state.waveform_cache_stats {
        if wf_count > 0 || wf_bytes > 0 {
            rows_col = rows_col.push(mk_row(
                "Waveforms".to_string(),
                format_count(wf_count),
                String::new(),
                format_bytes(wf_bytes),
            ));
        }
    }

    // Totals on the right of a final row.
    let mut total_bytes = 0u64;
    if let Some((_, art_bytes)) = state.artwork.cache_stats { total_bytes += art_bytes; }
    if let Some((lib_bytes, _)) = &state.library_cache_stats { total_bytes += *lib_bytes; }
    if let Some((_, wf_bytes)) = state.waveform_cache_stats { total_bytes += wf_bytes; }
    let totals_row: Element<'_, GuiMessage> = if total_bytes > 0 {
        container(text(format!("Total on disk: {}", format_bytes(total_bytes))).size(13))
            .padding([2, 6])
            .into()
    } else {
        container(text("").size(13)).padding([2, 6]).into()
    };

    column![text("Cache").size(15), rows_col, totals_row]
        .spacing(4)
        .into()
}

/// Human-readable byte size — KB / MB / GB to one decimal place,
/// integer for sub-KB. Matches the way other clients (Plexamp,
/// macOS Finder) format storage figures, so users get a familiar
/// "x.x MB" without the table forcing a wider Size column.
fn format_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let bf = b as f64;
    if bf >= GB {
        format!("{:.1} GB", bf / GB)
    } else if bf >= MB {
        format!("{:.1} MB", bf / MB)
    } else if bf >= KB {
        format!("{:.1} KB", bf / KB)
    } else {
        format!("{} B", b)
    }
}

fn format_count(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn textamp_section<'a>(state: &'a AppState, ui_scale: f32) -> Element<'a, GuiMessage> {
    let current_theme = state.theme;
    let theme_rows: Vec<Element<'a, GuiMessage>> = ThemeName::all().iter().map(|&t| {
        let active = t == current_theme;
        button(text(t.display_name()).size(14))
            .width(Length::Fixed(160.0))
            .padding([3, 10])
            .on_press(GuiMessage::SetTheme(t))
            .style(move |theme: &Theme, status: button::Status| {
                let p = theme.extended_palette();
                let (bg, fg) = if active {
                    (p.primary.strong.color, p.primary.strong.text)
                } else {
                    match status {
                        button::Status::Hovered => (p.primary.weak.color, p.primary.strong.color),
                        _ => (p.background.weak.color, p.background.base.text),
                    }
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: fg,
                    border: Border { color: p.background.strong.color, width: 1.0, radius: 3.0.into() },
                    ..button::Style::default()
                }
            })
            .into()
    }).collect();

    let theme_section = column![
        text("Theme").size(16),
        Column::with_children(theme_rows).spacing(4),
    ]
    .spacing(6);

    let minus_enabled = ui_scale > UI_SCALE_MIN + f32::EPSILON;
    let plus_enabled = ui_scale < UI_SCALE_MAX - f32::EPSILON;
    let minus = scale_btn("-", GuiMessage::AdjustUiScale(-UI_SCALE_STEP), minus_enabled);
    let plus = scale_btn("+", GuiMessage::AdjustUiScale(UI_SCALE_STEP), plus_enabled);
    let reset = button(text("Reset").size(14))
        .padding([4, 12])
        .on_press(GuiMessage::AdjustUiScale(1.0 - ui_scale))
        .style(popout_button_style);
    let scale_row = iced_row![
        text("UI scale").size(15),
        minus,
        text(format!("{:.2}x", ui_scale)).size(15),
        plus,
        reset,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    // Volume — used to live in the transport bar; moved here so the
    // transport stays compact (the user adjusts volume rarely from
    // the GUI, and Ctrl+Shift+↑/↓ keyboard shortcuts still work
    // regardless of the slider's location).
    use iced::widget::slider;
    use crate::app::action::PlaybackAction;
    let vol = state.playback.volume.clamp(0.0, 1.0);
    let vol_pct = (vol * 100.0).round() as u32;
    let vol_slider = slider(0.0..=1.0, vol, |v| {
        GuiMessage::Action(Action::Playback(PlaybackAction::SetVolume(v)))
    })
    .step(0.01_f32)
    .width(Length::Fixed(220.0));
    let vol_row = iced_row![
        text("Volume").size(15),
        vol_slider,
        text(format!("{vol_pct}%")).size(14),
        button(text("Mute / Unmute").size(14))
            .padding([4, 12])
            .on_press(GuiMessage::Action(Action::Playback(PlaybackAction::ToggleMute)))
            .style(popout_button_style),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    use crate::services::external_search::SearchTarget;
    use iced::widget::checkbox;
    let ext = state.external_search;
    // Custom row: iced's default `Checkbox` puts the label after the
    // square but theme + container text colour interact poorly here
    // (label rendered same shade as the popup background, looking
    // like an empty box). Render the label as a sibling `text` so it
    // always paints with the popup body's foreground colour.
    let cb = |label: &'static str, on: bool, target: SearchTarget| -> Element<'static, GuiMessage> {
        let toggle = move |_| GuiMessage::Action(Action::Settings(
            SettingsAction::ToggleExternalSearchService(target),
        ));
        iced_row![
            checkbox("", on)
                .size(16)
                .on_toggle(toggle),
            text(label).size(15),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    };
    let ext_section = column![
        text("Search in external services").size(16),
        cb("Apple Music", ext.apple_music, SearchTarget::AppleMusic),
        cb("Spotify",     ext.spotify,     SearchTarget::Spotify),
        cb("YouTube",     ext.youtube,     SearchTarget::YouTube),
    ]
    .spacing(6);

    container(
        column![
            theme_section,
            Space::with_height(Length::Fixed(10.0)),
            text("View Options").size(16),
            scale_row,
            Space::with_height(Length::Fixed(10.0)),
            text("Playback").size(16),
            vol_row,
            Space::with_height(Length::Fixed(10.0)),
            ext_section,
        ]
        .spacing(8),
    )
    .width(Length::Fill)
    .into()
}

fn action_btn(label: &'static str, action: Action) -> Element<'static, GuiMessage> {
    button(text(label).size(14))
        .padding([4, 12])
        .on_press(GuiMessage::Action(action))
        .style(popout_button_style)
        .into()
}

fn scale_btn<'a>(label: &'a str, msg: GuiMessage, enabled: bool) -> Element<'a, GuiMessage> {
    let btn = button(text(label).size(16)).padding([2, 12]).style(popout_button_style);
    if enabled { btn.on_press(msg).into() } else { btn.into() }
}

/// Legacy entry used before the settings popup — unused now; leave
/// the function signature stable in case callers return.
#[allow(dead_code)]
pub fn scrollable_view<'a>(state: &'a AppState, ui_scale: f32) -> Element<'a, GuiMessage> {
    scrollable(view(state, ui_scale))
        .direction(crate::ui_gui::widgets::fat_vertical_scrollbar())
        .style(crate::ui_gui::widgets::chunky_scrollable_style)
        .height(Length::Fill)
        .into()
}
