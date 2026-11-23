//! Native menu coverage for the shared global palette registry.
use super::{Menu, MenuItem, NativeCommand, PaletteCommandKind, RadioCommand};
use crate::app::state::PaletteEntry;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewOptions {
    pub application_mode: Option<crate::app::launch::ApplicationMode>,
    pub covers: bool,
    pub large_covers: bool,
    pub compilation_artists: bool,
    pub grouped: bool,
    pub reverse: bool,
    pub sort: Option<crate::app::state::ColumnSortMode>,
}
pub fn view_options(state: &crate::app::AppState) -> ViewOptions {
    let column = state.browse_nav().and_then(|nav| nav.focused());
    let checked = |id| crate::app::view_options::is_checked(state, column, id);
    use crate::app::state::SortPopupOption;
    ViewOptions {
        application_mode: state.application_mode(),
        covers: checked(SortPopupOption::Artwork),
        large_covers: checked(SortPopupOption::LargeArtwork),
        compilation_artists: checked(SortPopupOption::CompilationOnlyArtists),
        grouped: checked(SortPopupOption::GroupByAlbum),
        reverse: checked(SortPopupOption::Direction),
        sort: column.map(|column| column.sort_mode),
    }
}
pub fn check_view_options(menus: &mut [Menu], options: &ViewOptions) {
    for menu in menus {
        for item in &mut menu.items {
            match item {
                MenuItem::Submenu(menu) => check_view_options(std::slice::from_mut(menu), options),
                MenuItem::Action {
                    action,
                    checked,
                    disabled,
                    ..
                } => {
                    if action.as_any().is::<super::LargeAlbumCovers>() {
                        *checked = options.large_covers;
                    }
                    let command =
                        if let Some(command) = action.as_any().downcast_ref::<NativeCommand>() {
                            Some(command.command())
                        } else if let Some(command) = action
                            .as_any()
                            .downcast_ref::<super::menu_catalog::Invoke>()
                        {
                            super::menu_catalog::resolve(
                                &command.key,
                                &super::menu_catalog::catalog(None),
                            )
                        } else {
                            None
                        };
                    if let Some(command) = command {
                        use PaletteCommandKind::*;
                        let value = match command {
                            SetApplicationMode(mode) => {
                                Some(options.application_mode == Some(mode))
                            }
                            ToggleArtwork => Some(options.covers),
                            ToggleCompilationArtists => Some(options.compilation_artists),
                            ToggleGroupByAlbum => Some(options.grouped),
                            ReverseSort => Some(options.reverse),
                            ApplySort(mode) => Some(options.sort == Some(mode)),
                            _ => None,
                        };
                        if let Some(value) = value {
                            *checked = value;
                        }
                        if matches!(command, CycleApplicationMode) {
                            *disabled = options.application_mode.is_none();
                        } else if matches!(command, SetApplicationMode(_)) {
                            *disabled = *checked;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn entries() -> Vec<PaletteEntry> {
    crate::app::command_palette::global_entries()
}

use crate::app::command_catalog::menu as group;

pub(super) fn disabled_queue_menu_items(state: &crate::app::AppState) -> Vec<&'static str> {
    static QUEUE_COMMANDS: std::sync::OnceLock<Vec<(String, PaletteCommandKind)>> =
        std::sync::OnceLock::new();
    let entries = QUEUE_COMMANDS.get_or_init(|| {
        entries()
            .into_iter()
            .filter(|entry| group(&entry.command) == "Queue")
            .map(|entry| (entry.label, entry.command))
            .chain(
                [
                    ("Save Queue as Playlist…", PaletteCommandKind::SaveQueue),
                    ("Auto-Mix", PaletteCommandKind::ToggleAutoMix),
                    (
                        "Remove from Queue",
                        PaletteCommandKind::RemoveFocusedFromQueue,
                    ),
                    ("Clear Queue", PaletteCommandKind::ClearQueue),
                    ("Undo Queue Edit", PaletteCommandKind::UndoQueueEdit),
                ]
                .into_iter()
                .map(|(label, command)| (label.into(), command)),
            )
            .collect()
    });
    entries
        .iter()
        .filter_map(|(label, command)| {
            crate::app::command_catalog::disabled_reason(state, command).map(|_| label.as_str())
        })
        .collect()
}

pub fn add_radio_menu(menus: &mut [Menu], radio: &[String]) {
    if radio.is_empty() {
        return;
    }
    let mut radio = radio.to_vec();
    radio.sort_by_key(|label| match label.trim_start_matches("Radio: ") {
        "Play Random Albums" => 0,
        "Random Artist Mix" => 1,
        "Manual Queue" => 3,
        _ => 2,
    });
    let queue = menus
        .iter_mut()
        .find(|m| m.name.as_ref() == "Queue")
        .expect("Queue menu");
    if queue
        .items
        .iter()
        .any(|item| matches!(item,MenuItem::Submenu(menu) if menu.name.as_ref()=="Queue Type"))
    {
        return;
    }
    queue.items.push(MenuItem::submenu(Menu {
        name: "Queue Type".into(),
        disabled: false,
        items: radio
            .iter()
            .map(|name| {
                MenuItem::action(
                    crate::app::command_catalog::station_label(name),
                    RadioCommand { name: name.clone() },
                )
            })
            .collect(),
    }));
}

#[cfg(test)]
use crate::app::command_catalog::submenu_for;

/// Keep common actions at the top level and preserve each action and checkmark
/// while grouping the less frequent command families.

#[cfg(test)]
mod tests {
    use super::super::{Library, NowPlaying, PlayPause, Quit, Settings};
    use super::*;
    #[test]
    fn random_album_navigation_is_a_direct_discover_command_with_a_mac_shortcut() {
        assert_eq!(group(&PaletteCommandKind::GoToRandomAlbum), "Discover");
        assert_eq!(
            submenu_for("Discover", &PaletteCommandKind::GoToRandomAlbum),
            None
        );
        assert_eq!(
            super::super::palette::shortcut(&PaletteCommandKind::GoToRandomAlbum),
            Some("cmd-shift-r")
        );
    }
    #[test]
    fn every_global_palette_definition_has_a_native_menu_command() {
        fn collect(menus: &[Menu], commands: &mut Vec<String>) {
            for menu in menus {
                for item in &menu.items {
                    match item {
                        MenuItem::Submenu(menu) => collect(std::slice::from_ref(menu), commands),
                        MenuItem::Action { action, .. } => {
                            let command = if let Some(invoke) = action
                                .as_any()
                                .downcast_ref::<super::super::menu_catalog::Invoke>(
                            ) {
                                super::super::menu_catalog::resolve(
                                    &invoke.key,
                                    &super::super::menu_catalog::catalog(None),
                                )
                            } else if let Some(native) =
                                action.as_any().downcast_ref::<NativeCommand>()
                            {
                                Some(native.command())
                            } else if action.as_any().is::<gpui_kit::component::input::Undo>() {
                                Some(PaletteCommandKind::UndoQueueEdit)
                            } else if action.as_any().is::<Library>() {
                                Some(PaletteCommandKind::GotoLibrary)
                            } else if action.as_any().is::<NowPlaying>() {
                                Some(PaletteCommandKind::GotoNowPlaying)
                            } else if action.as_any().is::<Settings>() {
                                Some(PaletteCommandKind::OpenSettings)
                            } else if action.as_any().is::<Quit>() {
                                Some(PaletteCommandKind::Quit)
                            } else if action.as_any().is::<PlayPause>() {
                                Some(PaletteCommandKind::PlayPause)
                            } else {
                                None
                            };
                            if let Some(command) = command {
                                commands.push(format!("{command:?}"));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut actual = vec![];
        collect(&super::super::native_menus(&[]), &mut actual);
        for entry in entries() {
            assert!(
                actual.contains(&format!("{:?}", entry.command)),
                "missing menu command: {}",
                entry.label
            );
        }
    }
    #[test]
    fn submenu_labels_remove_namespaces_without_changing_command_identity() {
        let menus = super::super::native_menus(&["Radio: Library Mix".into()]);
        fn check(menus: &[Menu]) {
            for menu in menus {
                for item in &menu.items {
                    match item {
                        MenuItem::Submenu(child) => check(std::slice::from_ref(child)),
                        MenuItem::Action { name, action, .. } => {
                            assert!(
                                !["Radio: ", "Remix: ", "Sort: ", "AudioMuse: "]
                                    .iter()
                                    .any(|prefix| name.starts_with(prefix))
                            );
                            if menu.name == "Radio Stations"
                                && action.as_any().is::<super::super::RadioCommand>()
                            {
                                assert_eq!(name.as_ref(), "Library");
                                assert_eq!(
                                    action
                                        .as_any()
                                        .downcast_ref::<super::super::RadioCommand>()
                                        .unwrap()
                                        .name,
                                    "Radio: Library Mix"
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        check(&menus);
    }

    #[test]
    fn radio_and_layout_choices_have_one_consistent_menu_location() {
        let menus = super::super::native_menus(&[
            "Radio: Library Mix".into(),
            "Radio: Random Artist Mix".into(),
            "Radio: Play Random Albums".into(),
        ]);
        let discover = menus.iter().find(|menu| menu.name == "Discover").unwrap();
        assert!(!discover.items.iter().any(
            |item| matches!(item, MenuItem::Action {name,..} if name == "Random Artist Mix")
        ));
        assert!(
            !discover
                .items
                .iter()
                .any(|item| matches!(item, MenuItem::Submenu(menu) if menu.name == "Collections"))
        );
        let queue = menus.iter().find(|menu| menu.name == "Queue").unwrap();
        let radio = queue
            .items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(menu) if menu.name == "Queue Type" => Some(menu),
                _ => None,
            })
            .unwrap();
        let names: Vec<_> = radio
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.as_ref()),
                _ => None,
            })
            .collect();
        assert!(
            names
                .windows(2)
                .any(|pair| pair == ["Play Random Albums", "Random Artist Mix"])
        );
        let view = menus.iter().find(|menu| menu.name == "View").unwrap();
        let fullscreen = view
            .items
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu(menu) if menu.name == "Full Screen" => Some(menu),
                _ => None,
            })
            .unwrap();
        let names: Vec<_> = fullscreen
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            [
                "Artwork",
                "Artwork with Transport",
                "Visualizer",
                "Visualizer with Transport",
                "Toggle Full Screen"
            ]
        );
        let window = menus.iter().find(|menu| menu.name == "Window").unwrap();
        assert!(
            !window
                .items
                .iter()
                .any(|item| matches!(item, MenuItem::Submenu(menu) if menu.name == "Window Mode"))
        );
    }

    #[test]
    fn native_menus_disable_queue_edits_until_tracks_are_available() {
        fn is_disabled(menu: &Menu, key: &str) -> Option<bool> {
            menu.items.iter().find_map(|item| match item {
                MenuItem::Submenu(menu) => is_disabled(menu, key),
                MenuItem::Action {
                    action, disabled, ..
                } => action
                    .as_any()
                    .downcast_ref::<super::super::menu_catalog::Invoke>()
                    .filter(|invoke| invoke.key == key)
                    .map(|_| *disabled),
                _ => None,
            })
        }

        let mut state = crate::app::AppState::new();
        for (track_count, expected) in [(0, true), (2, false)] {
            state.queue.tracks = vec![Default::default(); track_count];
            let snapshot = crate::desktop::engine::Snapshot {
                menu_commands: crate::desktop::menu_catalog::catalog(Some(&state)),
                ..Default::default()
            };
            let menus = crate::desktop::native_menus_for_snapshot(&snapshot);
            for command in [
                PaletteCommandKind::RemixShuffle,
                PaletteCommandKind::ToggleAutoMix,
                PaletteCommandKind::ClearQueue,
            ] {
                let key = crate::app::command_history::key(&command).unwrap();
                assert_eq!(
                    menus.iter().find_map(|menu| is_disabled(menu, &key)),
                    Some(expected),
                    "{command:?}"
                );
            }
        }
    }

    #[test]
    fn file_menu_can_create_a_playlist_without_a_track_selection() {
        let menus = super::super::native_menus(&[]);
        let file = menus.iter().find(|menu| menu.name == "File").unwrap();
        assert!(file.items.iter().any(|item| matches!(item, MenuItem::Action { name, action, .. }
            if name == "Create New Playlist…" && action.as_any().downcast_ref::<super::super::menu_catalog::Invoke>()
                .is_some_and(|invoke| invoke.key == "NewPlaylist"))));
    }

    #[test]
    fn native_menus_keep_long_command_families_in_submenus() {
        let menus = super::super::native_menus(&[]);
        let view = menus.iter().find(|menu| menu.name == "View").unwrap();
        for title in ["Sort", "View Options", "Browse", "Full Screen"] {
            assert!(
                view.items
                    .iter()
                    .any(|item| matches!(item, MenuItem::Submenu(menu) if menu.name == title)),
                "missing {title}"
            );
        }
        let queue = menus.iter().find(|menu| menu.name == "Queue").unwrap();
        for title in ["Queue Edits", "Reorder", "Saved Queue"] {
            assert!(
                queue
                    .items
                    .iter()
                    .any(|item| matches!(item, MenuItem::Submenu(menu) if menu.name == title)),
                "missing {title}"
            );
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn multi_window_menu_has_a_standalone_visualizer_destination() {
        let mut snapshot = crate::desktop::Snapshot {
            presentation: crate::app::windowed_presentation::Presentation::windowed(),
            ..Default::default()
        };
        for multi in [false, true] {
            snapshot.separate_player = multi;
            let menus = crate::desktop::native_menus_for_snapshot(&snapshot);
            let window = menus.iter().find(|menu| menu.name == "Window").unwrap();
            assert_eq!(window.items.iter().any(|item| matches!(item, MenuItem::Action {name,action,..} if name == "Visualizer" && action.as_any().is::<crate::desktop::Visualizer>())), multi);
        }
    }
    #[test]
    fn modes_submenu_checks_only_the_current_mode_and_preserves_the_cycle_shortcut() {
        for mode in crate::app::launch::ApplicationMode::ALL {
            let mut menus = super::super::native_menus(&[]);
            check_view_options(
                &mut menus,
                &ViewOptions {
                    application_mode: Some(mode),
                    ..Default::default()
                },
            );
            let view = menus.iter().find(|menu| menu.name == "View").unwrap();
            let modes = view
                .items
                .iter()
                .find_map(|item| match item {
                    MenuItem::Submenu(menu) if menu.name == "Modes" => Some(menu),
                    _ => None,
                })
                .unwrap();
            for choice in crate::app::launch::ApplicationMode::ALL {
                assert!(modes.items.iter().any(|item| matches!(item, MenuItem::Action { name, checked, disabled, .. } if name == choice.label() && *checked == (choice == mode) && *disabled == (choice == mode))));
            }
            assert!(view.items.iter().any(|item| matches!(item, MenuItem::Action { name, action, disabled:false, .. } if name == "Cycle Application Mode" && action.as_any().downcast_ref::<super::super::menu_catalog::Invoke>().is_some_and(|invoke|invoke.key=="CycleApplicationMode"))));
        }
    }
    #[test]
    fn native_menu_labels_use_the_shared_palette_names() {
        fn check(menus: &[Menu]) {
            for menu in menus {
                for item in &menu.items {
                    match item {
                        MenuItem::Submenu(menu) => check(std::slice::from_ref(menu)),
                        MenuItem::Action { name, action, .. } => {
                            if let Some(native) = action.as_any().downcast_ref::<NativeCommand>() {
                                let command = native.command();
                                assert_eq!(
                                    name.as_ref(),
                                    crate::app::command_catalog::label(&command, name.as_ref())
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        check(&super::super::native_menus(&[]));
    }
}
