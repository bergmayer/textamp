//! Preference values projected from the same catalog used by the terminal frontend.
use crate::app::{
    AppState,
    state::{SettingsPage, TextampSetting},
};
#[derive(Clone)]
pub struct Item {
    pub id: TextampSetting,
    pub description: &'static str,
    pub group: &'static str,
    pub value_kind: crate::app::preferences::Value,
    pub label: String,
    pub value: String,
    pub checked: Option<bool>,
}
#[derive(Clone)]
pub struct Page {
    pub page: SettingsPage,
    pub items: Vec<Item>,
}
pub fn project(state: &AppState) -> Vec<Page> {
    SettingsPage::ALL
        .into_iter()
        .map(|page| Page {
            page,
            items: crate::app::preferences::page(state, page)
                .into_iter()
                .filter(|item| !matches!(item.id, TextampSetting::LibraryRootVisible(_)))
                .map(|item| Item {
                    id: item.id,
                    description: item.description,
                    group: item.group,
                    label: item.label,
                    value: item.value.display(),
                    checked: item.value.checked(),
                    value_kind: item.value,
                })
                .collect(),
        })
        .collect()
}

pub use crate::app::sidebar::{SidebarOption, SidebarTarget, sidebar_options};

#[cfg(test)]
mod tests {
    use super::*;
    fn navidrome_state() -> AppState {
        let source = crate::navidrome::Source {
            id: "fixture".into(),
            name: "Music".into(),
            url: "http://127.0.0.1:1".into(),
            username: "fixture".into(),
            libraries: vec![],
        };
        let mut state = AppState::new();
        state.sources.active = crate::app::sources::ActiveSource::Navidrome(Box::new(
            crate::app::sources::navidrome::Session {
                client: crate::navidrome::Client::new(&source, "fixture-password".into(), None)
                    .unwrap(),
                source,
                extensions: Default::default(),
            },
        ));
        state
    }
    #[test]
    fn sidebar_settings_follow_rendered_destinations_and_album_order() {
        let state = navidrome_state();
        let choices = sidebar_options(&state);
        let settings: Vec<_> = choices
            .iter()
            .filter(|option| {
                option.group != "Playlists"
                    && option.group != "Radio Stations"
                    && option.group != "AI"
            })
            .map(|option| option.label)
            .collect();
        let rows = super::super::catalog::sidebar(&state);
        let rendered: Vec<_> = rows
            .iter()
            .filter(|row| !row.header && row.label != "Search…" && row.label != "Journey…")
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(settings, rendered);
        assert_eq!(
            &settings[1..8],
            &[
                "All",
                "Random",
                "Favourites",
                "Top Rated",
                "Recently Added",
                "Recently Played",
                "Most Played"
            ]
        );
        assert!(!choices.iter().any(|option| option.label == "Library"));
    }
    #[test]
    fn hidden_sidebar_choices_remain_available_to_restore() {
        use crate::app::sources::navidrome::commands::CollectionKind;
        use crate::app::state::{BrowseCategory, LibraryRoot, SidebarSection};
        let mut state = navidrome_state();
        state.desktop_hidden_library_roots.push(LibraryRoot::Albums);
        state.hidden_sections.push(BrowseCategory::Playlists);
        state
            .hidden_collections
            .extend([CollectionKind::RandomAlbums, CollectionKind::TopRatedAlbums]);
        let choices = sidebar_options(&state);
        for target in [
            SidebarTarget::Root(LibraryRoot::Albums),
            SidebarTarget::Section(SidebarSection::Category(BrowseCategory::Playlists)),
            SidebarTarget::Section(SidebarSection::Collection(CollectionKind::RandomAlbums)),
            SidebarTarget::Section(SidebarSection::Collection(CollectionKind::TopRatedAlbums)),
        ] {
            assert!(
                !choices
                    .iter()
                    .find(|option| option.target == target)
                    .unwrap()
                    .checked
            );
        }
        let rows = super::super::catalog::sidebar(&state);
        assert!(rows.iter().any(|row| row.header && row.label == "Albums"));
        assert!(
            !rows
                .iter()
                .any(|row| ["All", "Random", "Top Rated"].contains(&row.label.as_str()))
        );
        assert!(rows.iter().any(|row| row.label == "Recently Added"));
    }
    #[test]
    fn audio_preferences_keep_shared_identity_without_the_output_picker() {
        let state = AppState::new();
        let pages = project(&state);
        let page = pages
            .iter()
            .find(|page| page.page == SettingsPage::Audio)
            .unwrap();
        assert!(page.items.iter().all(|item| item.label != "Sound Output"));
        let item = page
            .items
            .iter()
            .find(|item| item.label == "Allow same artist")
            .unwrap();
        assert!(matches!(item.id, TextampSetting::RemixAllowSameArtist));
    }
}
