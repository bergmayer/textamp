//! Canonical preference identity, presentation metadata and typed values.
use super::{AppState, state::*};
use crate::config::Config;

/// Representative library content shared by theme previews.
pub const THEME_PREVIEW_ALBUM: &str = "A favorite album";
/// Sample titles and durations displayed by both theme previews.
pub const THEME_PREVIEW_TRACKS: [(&str, &str); 2] =
    [("First track", "3:24"), ("Second track", "4:08")];

pub const TRANSCODE_BITRATES: [u32; 5] = [0, 128, 192, 256, 320];

/// Persistence boundary of a preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Application,
    Library,
}

/// Values distinguish independent switches, exclusive choices and bounded counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Toggle {
        current: bool,
        default: bool,
    },
    Choice {
        selected: bool,
        default: bool,
    },
    Count {
        current: u32,
        default: u32,
        min: u32,
        max: u32,
    },
    Text {
        current: String,
        default: String,
    },
}
impl Value {
    /// Checked state for a switch or one member of an exclusive choice.
    pub fn checked(&self) -> Option<bool> {
        match self {
            Self::Toggle { current, .. } => Some(*current),
            Self::Choice { selected, .. } => Some(*selected),
            _ => None,
        }
    }
    /// Display value without frontend-specific activation hints.
    pub fn display(&self) -> String {
        match self {
            Self::Toggle { .. } | Self::Choice { .. } => String::new(),
            Self::Count { current, .. } => current.to_string(),
            Self::Text { current, .. } => current.clone(),
        }
    }
}

/// One shared preference projected by each frontend's settings components.
#[derive(Clone, Debug)]
pub struct Preference {
    pub id: TextampSetting,
    pub label: String,
    pub group: &'static str,
    pub description: &'static str,
    pub scope: Scope,
    pub value: Value,
}

/// Available preferences in canonical page order, filtered by source capabilities.
pub fn page(state: &AppState, page: SettingsPage) -> Vec<Preference> {
    let defaults = Config::default();
    state
        .textamp_settings_for(page)
        .into_iter()
        .map(|id| describe(state, id, &defaults))
        .collect()
}

fn describe(state: &AppState, id: TextampSetting, defaults: &Config) -> Preference {
    use crate::services::external_search::SearchTarget;
    use TextampSetting::*;
    let (label, group, description, scope, value) = match id {
        Theme(name) => (
            name.display_name().into(),
            "Theme",
            "",
            Scope::Application,
            Value::Choice {
                selected: state.theme == name,
                default: defaults.ui.theme == name.config_name(),
            },
        ),
        Presentation(mode) => (
            mode.label().into(),
            "Mode",
            "",
            Scope::Application,
            Value::Choice {
                selected: state.application_mode() == Some(mode),
                default: mode == super::launch::ApplicationMode::Gui,
            },
        ),
        Artwork(mode) => (
            match mode {
                ArtworkMode::Auto => "Images",
                ArtworkMode::Halfblocks => "ANSI half blocks",
                ArtworkMode::Braille => "Braille",
            }
            .into(),
            "Artwork",
            "",
            Scope::Library,
            Value::Choice {
                selected: state.artwork_mode() == mode,
                default: ArtworkMode::from_config(&defaults.ui.artwork_mode) == mode,
            },
        ),
        RemixAllowSameArtist => (
            "Allow same artist".into(),
            "Queue tools",
            "Queue tools may add another album by the same artist. Twofer always allows the same artist.",
            Scope::Application,
            Value::Toggle {
                current: state.remix_allow_same_artist,
                default: defaults.ui.remix_allow_same_artist,
            },
        ),
        SinglesTrackLimit => (
            "Singles: maximum tracks".into(),
            "Singles",
            "Albums with this many tracks or fewer appear in Singles.",
            Scope::Application,
            Value::Count {
                current: state.singles_track_limit,
                default: defaults.ui.singles_track_limit,
                min: 1,
                max: crate::services::singles::MAX_LIMIT,
            },
        ),
        Transcode => (
            "Streaming quality".into(),
            "Streaming",
            "Original audio or a bitrate limit, starting with the next track.",
            Scope::Application,
            Value::Text {
                current: if state.transcode_kbps == 0 {
                    "Original audio".into()
                } else {
                    format!("Up to {} kbps", state.transcode_kbps)
                },
                default: "Original audio".into(),
            },
        ),
        WebSearch(provider) => (
            provider.name().into(),
            "Web search",
            "Open artist searches in your external browser.",
            Scope::Application,
            Value::Choice {
                selected: state.web_search_provider == provider,
                default: defaults.ui.web_search_provider == provider,
            },
        ),
        ExternalSearch(target) => {
            let (name, current, default) = match target {
                SearchTarget::AppleMusic => (
                    "Apple Music",
                    state.external_search.apple_music,
                    defaults.ui.enable_apple_music_search,
                ),
                SearchTarget::Spotify => (
                    "Spotify",
                    state.external_search.spotify,
                    defaults.ui.enable_spotify_search,
                ),
                SearchTarget::YouTube => (
                    "YouTube",
                    state.external_search.youtube,
                    defaults.ui.enable_youtube_search,
                ),
            };
            (
                name.into(),
                "Search services",
                "Include this service in search commands.",
                Scope::Application,
                Value::Toggle { current, default },
            )
        }
        LibraryRootVisible(root) => (
            root.label().into(),
            "Show / Hide",
            "Show this item in library navigation.",
            Scope::Application,
            Value::Toggle {
                current: super::sidebar::library_root_visible(state, root),
                default: !defaults.ui.desktop_hidden_library_roots.contains(&root),
            },
        ),
        Sidebar(section) => (
            section.label().into(),
            "Show / Hide",
            "Show this item in library navigation.",
            Scope::Application,
            Value::Toggle {
                current: !section.hidden(state),
                default: match section {
                    SidebarSection::Category(category) => {
                        !defaults.ui.hidden_sections.contains(&category)
                    }
                    SidebarSection::Collection(kind) => {
                        !defaults.ui.hidden_collections.contains(&kind)
                    }
                },
            },
        ),
    };
    Preference {
        id,
        label,
        group,
        description,
        scope,
        value,
    }
}

/// Canonical settings page for an identity, independent of either settings widget.
pub fn page_for(id: TextampSetting) -> SettingsPage {
    match id {
        TextampSetting::Theme(_) | TextampSetting::Presentation(_) | TextampSetting::Artwork(_) => {
            SettingsPage::Themes
        }
        TextampSetting::RemixAllowSameArtist | TextampSetting::Transcode => SettingsPage::Audio,
        TextampSetting::SinglesTrackLimit
        | TextampSetting::LibraryRootVisible(_)
        | TextampSetting::Sidebar(_) => SettingsPage::Sidebar,
        TextampSetting::ExternalSearch(_) | TextampSetting::WebSearch(_) => SettingsPage::Services,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn showing_one_destination_preserves_the_other_hidden_destinations() {
        let mut state = AppState::new();
        state.hidden_sections.push(BrowseCategory::Library);
        assert!(!super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Artists
        ));
        assert!(super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Downloads
        ));
        assert!(super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Radio
        ));
        super::super::sidebar::set_library_root_visible(&mut state, LibraryRoot::Artists, true);
        assert!(super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Artists
        ));
        assert!(!super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Albums
        ));
        assert!(!super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Tracks
        ));
        super::super::sidebar::set_library_root_visible(&mut state, LibraryRoot::Downloads, false);
        assert!(
            !super::super::sidebar::sidebar(&state)
                .iter()
                .any(|row| row.library_root == Some(LibraryRoot::Downloads))
        );
        assert!(super::super::sidebar::library_root_visible(
            &state,
            LibraryRoot::Artists
        ));
    }

    #[test]
    fn windowed_mode_choices_require_a_windowed_host() {
        let mut state = AppState::new();
        assert!(
            !page(&state, SettingsPage::Themes)
                .iter()
                .any(|item| matches!(item.id, TextampSetting::Presentation(_)))
        );
        state.windowed_presentation = super::super::windowed_presentation::Presentation::windowed();
        let modes: Vec<_> = page(&state, SettingsPage::Themes)
            .into_iter()
            .filter(|item| matches!(item.id, TextampSetting::Presentation(_)))
            .collect();
        assert_eq!(
            modes.len(),
            if cfg!(all(feature = "desktop", target_os = "macos")) {
                2
            } else {
                1
            }
        );
        assert_eq!(
            modes
                .iter()
                .filter(|item| item.value.checked() == Some(true))
                .count(),
            1
        );
    }
    #[test]
    fn identities_values_and_scope_stay_consistent_across_pages() {
        let mut state = AppState::new();
        state.active_library = Some("fixture".into());
        for category in SettingsPage::ALL {
            let preferences = page(&state, category);
            assert_eq!(
                preferences.iter().map(|p| p.id).collect::<Vec<_>>(),
                state.textamp_settings_for(category)
            );
            for preference in preferences {
                assert_eq!(page_for(preference.id), category);
                assert!(!preference.label.is_empty());
                assert!(!preference.description.is_empty());
                if let Value::Count {
                    current,
                    default,
                    min,
                    max,
                } = preference.value
                {
                    assert!((min..=max).contains(&current));
                    assert!((min..=max).contains(&default));
                }
            }
        }
        let artwork: Vec<_> = page(&state, SettingsPage::Themes)
            .into_iter()
            .filter(|item| matches!(item.id, TextampSetting::Artwork(_)))
            .collect();
        assert_eq!(artwork.len(), ArtworkMode::all().len());
        assert!(artwork.iter().all(|p| p.scope == Scope::Library));
    }
}
