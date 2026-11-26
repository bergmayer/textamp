//! Command matching and presentation metadata for the desktop Kit palette.
use super::surface::Item;
use crate::app::command_catalog::PalettePriority;
use crate::app::state::PaletteCommandKind;
use crate::services::CommandSearch;
use std::ops::Range;

pub(super) struct Match {
    pub index: usize,
    pub highlights: Vec<Range<usize>>,
}
pub(super) fn path(item: &Item) -> Vec<String> {
    use super::surface::Operation;
    match &item.operation {
        Operation::Palette(command) => crate::app::command_catalog::path(command),
        Operation::Native(native) => native.path.clone(),
        _ => vec![],
    }
}
pub(super) fn append_groups(items: &mut Vec<Item>, current: &[String]) {
    use super::surface::Operation;
    let groups = crate::app::command_catalog::groups(items.iter().map(path));
    for group in groups {
        items.push(Item {
            glyph: None,
            key: format!("group:{}", group.join("/")),
            label: group.last().unwrap().clone(),
            aliases: crate::app::command_catalog::group_aliases(&group),
            selected: false,
            operation: Operation::Palette(PaletteCommandKind::OpenCommandGroup(group)),
        });
    }
    if !current.is_empty() {
        items.push(Item {
            glyph: None,
            key: "palette-back".into(),
            label: "Back".into(),
            aliases: vec![],
            selected: false,
            operation: Operation::Palette(PaletteCommandKind::OpenCommandGroup(
                current[..current.len() - 1].to_vec(),
            )),
        });
    }
}
pub(super) fn matches(
    items: &[Item],
    query: &str,
    recent: &crate::app::command_history::History,
    current: &[String],
    disabled: &std::collections::BTreeMap<usize, &'static str>,
) -> Vec<Match> {
    let search = CommandSearch::new(query);
    let searching = !search.text().is_empty();
    let mut matches: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if let super::surface::Operation::Palette(command) = &item.operation {
                let available = !disabled.contains_key(&index);
                let promoted = current.is_empty()
                    && crate::app::command_catalog::palette_priority(command, available, recent)
                        != PalettePriority::Menu;
                if !(crate::app::command_catalog::visible(command, &item.label, current, searching)
                    || promoted)
                {
                    return None;
                }
            } else if !searching
                && path(item) != current
                && !(current.is_empty()
                    && !disabled.contains_key(&index)
                    && recent.rank(&item.key).is_some())
            {
                return None;
            }
            let usage = match &item.operation {
                super::surface::Operation::Palette(command) => {
                    crate::app::command_history::key(command).and_then(|key| recent.rank(&key))
                }
                _ => recent.rank(&item.key),
            }
            .map_or(0, |rank| 64 - rank);
            let score = if searching {
                Some(search.score(&item.label, item.aliases.iter().map(String::as_str))?)
            } else {
                None
            };
            let highlights = if searching {
                let chars: Vec<_> = item.label.char_indices().collect();
                search
                    .label_positions(&item.label)
                    .into_iter()
                    .filter_map(|index| {
                        chars
                            .get(index)
                            .map(|(offset, ch)| *offset..*offset + ch.len_utf8())
                    })
                    .collect()
            } else {
                vec![]
            };
            Some((score, usage, Match { index, highlights }))
        })
        .collect();
    if matches.is_empty() && let Some(index)=items.iter().position(|item|matches!(&item.operation,super::surface::Operation::Palette(c) if crate::app::sources::navidrome::playlist_edit::create_choice(c))) {
        return vec![Match {index,highlights:vec![]}];
    }
    matches.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| {
                let priority = |index: usize| match &items[index].operation {
                    super::surface::Operation::Palette(command) => {
                        crate::app::command_catalog::palette_priority(
                            command,
                            !disabled.contains_key(&index),
                            recent,
                        )
                    }
                    _ => recent
                        .rank(&items[index].key)
                        .filter(|_| !disabled.contains_key(&index))
                        .map_or(PalettePriority::Menu, PalettePriority::Recent),
                };
                if current.is_empty() {
                    priority(a.2.index).cmp(&priority(b.2.index))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.index.cmp(&b.2.index))
    });
    matches.into_iter().map(|(_, _, item)| item).collect()
}
pub(super) fn shortcut(command: &PaletteCommandKind) -> Option<&'static str> {
    use PaletteCommandKind::*;
    Some(match command {
        GotoLibrary => "cmd-1",
        GotoNowPlaying => "cmd-3",
        OpenSettings => "cmd-,",
        Quit => "cmd-q",
        OpenSearch => "cmd-f",
        GotoGenres => "cmd-g",
        GotoFolders => "cmd-shift-o",
        OpenHelp => "f1",
        Refresh => "f5",
        OpenSort => "f6",
        ToggleFullscreen => "f7",
        ClearQueue => "cmd-x",
        SaveQueue => "cmd-s",
        UndoQueueEdit => "cmd-z",
        SearchAppleMusic => "cmd-shift-a",
        OpenRadioBrowser => "cmd-alt-r",
        SonicAdventure => "cmd-alt-j",
        PlayRandomAlbums => "cmd-alt-a",
        LibraryMix => "cmd-alt-l",
        GoToRandomAlbum => "cmd-shift-r",
        PlayPause => "space",
        NextTrack => ">",
        PrevTrack => "<",
        ToggleFilter => "/",
        CloseColumn => "cmd-[",
        RemoveFocusedFromQueue => "delete",
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::super::surface::Operation;
    use super::*;
    fn item(label: &str, aliases: &[&str]) -> Item {
        Item {
            glyph: None,
            key: label.into(),
            label: label.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            selected: false,
            operation: Operation::Key(crossterm::event::KeyCode::Enter),
        }
    }
    #[test]
    fn root_promotes_only_available_recents_and_keeps_normal_routes() {
        let mut rows = [
            ("settings", "Settings…", PaletteCommandKind::OpenSettings),
            ("play", "Play Album", PaletteCommandKind::PlayFocusedAlbum),
            (
                "search",
                "Search Apple Music",
                PaletteCommandKind::SearchAppleMusic,
            ),
        ]
        .into_iter()
        .map(|(key, label, command)| Item {
            glyph: None,
            key: key.into(),
            label: label.into(),
            aliases: vec![],
            selected: false,
            operation: Operation::Palette(command),
        })
        .collect::<Vec<_>>();
        append_groups(&mut rows, &[]);
        let mut history = crate::app::command_history::History::default();
        history.remember("SearchAppleMusic".into());
        history.remember("PlayFocusedAlbum".into());
        let root = matches(&rows, "", &history, &[], &Default::default());
        assert_eq!(
            root.iter()
                .take(2)
                .map(|row| rows[row.index].key.as_str())
                .collect::<Vec<_>>(),
            ["play", "search"]
        );
        let disabled = [(2, "No music to search")].into_iter().collect();
        let root = matches(&rows, "", &history, &[], &disabled);
        assert!(!root.iter().any(|row| rows[row.index].key == "search"));
        let normal = matches(
            &rows,
            "",
            &history,
            &["Discover".into(), "Search".into()],
            &disabled,
        );
        assert!(normal.iter().any(|row| rows[row.index].key == "search"));
        let ordinary = matches(&rows, "", &Default::default(), &[], &Default::default());
        assert_eq!(rows[ordinary[0].index].key, "play");
        assert_eq!(rows[ordinary[1].index].key, "search");
    }

    #[test]
    fn fuzzy_matches_include_aliases_and_recent_commands_break_ties() {
        let rows = vec![
            item("Show Library", &["browse"]),
            item("Now Playing", &["player"]),
            item("Quit", &["q"]),
        ];
        assert_eq!(
            matches(&rows, "nwpl", &Default::default(), &[], &Default::default())[0].index,
            1
        );
        assert_eq!(
            matches(
                &rows,
                "browse",
                &Default::default(),
                &[],
                &Default::default()
            )[0]
            .index,
            0
        );
        assert_eq!(
            matches(
                &rows,
                "borwse",
                &Default::default(),
                &[],
                &Default::default()
            )[0]
            .index,
            0
        );
        assert_eq!(
            matches(&rows, "q", &Default::default(), &[], &Default::default())[0].index,
            2
        );
        let mut history = crate::app::command_history::History::default();
        history.remember("Quit".into());
        assert_eq!(
            matches(&rows, "", &history, &[], &Default::default())[0].index,
            2
        );
        assert_eq!(
            CommandSearch::new("  queue::::  clear_queue  ").text(),
            "queue: clear queue"
        );
    }
    #[test]
    fn highlighted_unicode_matches_stay_on_character_boundaries() {
        let rows = vec![item("Música: Play", &[])];
        let matched = matches(&rows, "úa", &Default::default(), &[], &Default::default());
        assert_eq!(matched.len(), 1);
        let highlighted: String = matched[0]
            .highlights
            .iter()
            .map(|range| &rows[0].label[range.clone()])
            .collect();
        assert_eq!(highlighted, "úa");
    }
}

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    use crate::desktop::surface::Operation;
    #[test]
    fn recent_menu_groups_never_displace_selection_actions() {
        let mut rows = vec![
            Item {
                glyph: None,
                key: "album-play".into(),
                label: "Play Album".into(),
                aliases: vec![],
                selected: false,
                operation: Operation::Palette(PaletteCommandKind::PlayFocusedAlbum),
            },
            Item {
                glyph: None,
                key: "queue".into(),
                label: "Play Queue".into(),
                aliases: vec![],
                selected: false,
                operation: Operation::Palette(PaletteCommandKind::GotoQueue),
            },
        ];
        append_groups(&mut rows, &[]);
        let visible = matches(&rows, "", &Default::default(), &[], &Default::default());
        assert_eq!(rows[visible[0].index].label, "Play Album");
        let normal = matches(
            &rows,
            "",
            &Default::default(),
            &["Playback".into(), "Selection".into()],
            &Default::default(),
        );
        assert!(
            normal
                .iter()
                .any(|row| rows[row.index].label == "Play Album")
        );
        assert!(!visible.iter().any(|row| rows[row.index].key == "queue"));
    }
    #[test]
    fn browsing_and_fuzzy_search_share_all_nested_commands() {
        let mut entry = crate::app::state::PaletteEntry {
            label: "Remix: Twins".into(),
            hint: String::new(),
            aliases: vec![],
            command: PaletteCommandKind::RemixGemini,
        };
        crate::app::command_catalog::prepare(&mut entry);
        let mut rows = vec![Item {
            glyph: None,
            key: "twins".into(),
            label: entry.label,
            selected: false,
            aliases: entry.aliases,
            operation: Operation::Palette(entry.command),
        }];
        let nested = vec!["Queue".into(), "Queue Edits".into()];
        append_groups(&mut rows, &nested);
        let root = matches(&rows, "", &Default::default(), &[], &Default::default());
        assert_eq!(root.len(), 1);
        assert_eq!(rows[root[0].index].label, "Queue");
        let children = matches(&rows, "", &Default::default(), &nested, &Default::default());
        assert!(children.iter().any(|row| rows[row.index].label == "Twins"));
        assert!(children.iter().any(|row| rows[row.index].label == "Back"));
        let found = matches(
            &rows,
            "queue twins",
            &Default::default(),
            &[],
            &Default::default(),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(rows[found[0].index].label, "Twins");
    }
}
