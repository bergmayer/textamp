//! Library-manager lifecycle, independent of the selected playback source.
use super::*;

pub fn reset_selection(state: &mut AppState) {
    state.popups.library_picker_index = 0;
    state.sources.picker_scroll_pin = None;
}

pub fn initialize(state: &mut AppState, config: &Config) {
    state.sources.audiomuse.connections = config.audiomuse_connections.clone();
    state.sources.sonic_disabled_libraries = config.sonic_disabled_libraries.clone();
    state.sources.folders = config.folder_sources.clone();
    state.sources.navidrome = config.navidrome_sources.clone();
    state.popups.library_picker_active = false;
    reset_selection(state);
}

pub fn startup_choice(config: &Config) -> Option<LibraryChoice> {
    if let Some(selection) = &config.default_navidrome {
        if let Some(source) = config
            .navidrome_sources
            .iter()
            .find(|s| s.id == selection.source_id)
        {
            return Some(LibraryChoice::Navidrome {
                source: source.clone(),
                folder: selection.folder.clone(),
                name: String::new(),
            });
        }
    }
    config
        .default_folder_source
        .as_ref()
        .and_then(|id| {
            config
                .folder_sources
                .iter()
                .find(|s| &s.id == id)
                .cloned()
                .map(LibraryChoice::Folder)
        })
        .or_else(|| {
            config
                .navidrome_sources
                .first()
                .map(|source| LibraryChoice::Navidrome {
                    source: source.clone(),
                    folder: source.canonical_folder(None),
                    name: String::new(),
                })
        })
        .or_else(|| {
            config
                .folder_sources
                .first()
                .cloned()
                .map(LibraryChoice::Folder)
        })
}

pub fn settings_active(state: &AppState) -> bool {
    state.view == View::Settings
        && state.settings_state.section == super::super::state::SettingsSection::Libraries
        && !state.popups.library_picker_active
}

impl LibraryChoice {
    /// Group by provider and sign-in, not by a library's display name.
    pub fn cache_store(&self) -> anyhow::Result<crate::library::cache::Store> {
        match self {
            Self::Navidrome { source, folder, .. } => navidrome_store(source, folder.clone()),
            Self::Folder(source) => crate::library::cache::Store::folder(source),
            _ => anyhow::bail!("Not a saved library"),
        }
    }
    pub fn group(&self) -> String {
        match self {
            Self::Navidrome { source, .. } => {
                format!("Navidrome · {} @ {}", source.username, source.url)
            }
            Self::Folder(source) => match &source.location {
                FolderLocation::Local { .. } => "Local folders".into(),
                FolderLocation::Webdav { url, username, .. } => {
                    let host = reqwest::Url::parse(url)
                        .ok()
                        .map(|u| u.origin().ascii_serialization())
                        .unwrap_or_default();
                    format!(
                        "WebDAV · {} @ {host}",
                        username.as_deref().unwrap_or("guest")
                    )
                }
            },
            _ => String::new(),
        }
    }

    pub fn provider(&self) -> &'static str {
        match self {
            Self::Folder(source) => source.kind_name(),
            Self::Navidrome { .. } | Self::AddNavidrome => "Navidrome",
            Self::Add => "",
            Self::AddFolder => "Local",
            Self::AddWebdav => "WebDAV",
        }
    }

    pub fn location(&self) -> String {
        match self {
            Self::Folder(source) => match &source.location {
                FolderLocation::Local { path } => path.display().to_string(),
                FolderLocation::Webdav { url, username, .. } => {
                    format!("{} @ {url}", username.as_deref().unwrap_or("guest"))
                }
            },
            Self::Navidrome { source, .. } => format!("{} @ {}", source.username, source.url),
            _ => String::new(),
        }
    }
}

pub fn navidrome_store(
    source: &crate::navidrome::Source,
    folder: Option<String>,
) -> anyhow::Result<crate::library::cache::Store> {
    crate::library::cache::Store::new((
        "navidrome",
        &source.id,
        source.url.trim_end_matches('/'),
        &source.username,
        source.canonical_folder(folder),
    ))
}
