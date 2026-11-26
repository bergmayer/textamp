//! Deterministic render and input checks without audio.
use super::{
    Command, Desktop, Engine, Snapshot, WindowRegistry, WindowRole, browser, catalog, init,
};
use crate::app::{
    AppState,
    state::{BrowseColumn, BrowseItem},
};
use gpui_kit::Focusable;
use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::AppContext;
use gpui_kit::gpui::{self, TestAppContext, VisualTestContext, px};

fn snapshot() -> Snapshot {
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "artists",
        vec![BrowseItem::Artist {
            key: "a".into(),
            title: "Artist A".into(),
            thumb: None,
            is_placeholder: false,
        }],
    )];
    Snapshot {
        queue_tools: crate::app::queue_controls::definitions(&state),
        columns: browser::columns(&state),
        section: "Library".into(),
        sidebar: vec![
            catalog::SidebarRow {
                parent: None,
                label: "Artists".into(),
                header: false,
                playlist_key: None,
                library_root: Some(crate::app::state::LibraryRoot::Artists),
                active: true,
                actions: vec![],
            },
            catalog::SidebarRow {
                parent: None,
                label: "Tracks".into(),
                header: false,
                playlist_key: None,
                library_root: Some(crate::app::state::LibraryRoot::Tracks),
                active: false,
                actions: vec![],
            },
            catalog::SidebarRow {
                parent: None,
                label: "Fixture Playlist".into(),
                header: false,
                playlist_key: Some("playlist".into()),
                library_root: None,
                active: false,
                actions: vec![],
            },
        ],
        ..Default::default()
    }
}
fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}
#[gpui::test]
fn sidebar_group_click_targets(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    let mut playlist = initial.sidebar.pop().unwrap();
    playlist.parent = Some(crate::app::sidebar::SidebarGroup::Playlists);
    let mut heading = playlist.clone();
    heading.parent = None;
    heading.header = true;
    heading.label = "Playlists".into();
    heading.playlist_key = None;
    initial.sidebar = vec![heading];
    initial
        .sidebar
        .extend(std::iter::repeat_n(playlist.clone(), 512));
    initial.collapsed_sidebar_sections = vec!["Playlists".into()];
    let mut long_group = initial.clone();
    long_group.sidebar.push(playlist);
    let (engine, updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    draw(cx);
    let arrow = cx.debug_bounds("sidebar-disclosure-Playlists").unwrap();
    cx.simulate_click(arrow.center(), Default::default());
    let actions: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(actions.iter().any(
        |cmd| matches!(cmd, Command::ToggleSidebarSection { title, .. } if title == "Playlists")
    ));
    assert!(
        !actions
            .iter()
            .any(|cmd| matches!(cmd, Command::SidebarGroup { .. }))
    );
    let mut label = arrow.center();
    label.x -= px(64.);
    cx.simulate_click(label, Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|cmd| matches!(cmd, Command::SidebarGroup { title, .. } if title == "Playlists"))
    );
    updates.send_replace(long_group);
    cx.run_until_parked();
    draw(cx);
    assert!(cx.debug_bounds("sidebar-disclosure-Playlists").is_none());
    cx.simulate_click(label, Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|cmd| matches!(cmd, Command::SidebarGroup { title, .. } if title == "Playlists"))
    );
}
#[gpui::test]
fn main_sidebar_opens_mp3_player_management(cx: &mut TestAppContext) {
    cx.update(init);
    let state = AppState::new();
    let mut initial = snapshot();
    initial.sidebar.extend(
        catalog::sidebar(&state)
            .into_iter()
            .filter(|row| row.library_root == Some(crate::app::state::LibraryRoot::Mp3Players)),
    );
    let index = initial
        .sidebar
        .iter()
        .position(|row| row.library_root == Some(crate::app::state::LibraryRoot::Mp3Players))
        .unwrap();
    let (engine, updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let first = initial.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = first;
        view
    });
    draw(cx);
    draw(cx);
    let item = cx
        .debug_bounds("sidebar-Audio Player")
        .expect("Audio Player must be in the main sidebar");
    cx.simulate_click(item.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command, Command::SidebarOpen { index: target, .. } if target == index
        ))
    );
    let mut next = initial.clone();
    next.player_sync.visible = true;
    next.player_sync.phase = crate::app::player_sync::Phase::Copying(Default::default());
    updates.send_replace(next);
    cx.run_until_parked();
    draw(cx);
    assert!(cx.debug_bounds("player-devices-scroll").is_some());
    for (target, selector) in [
        "sidebar-Artists",
        "sidebar-Tracks",
        "sidebar-Fixture Playlist",
    ]
    .into_iter()
    .enumerate()
    {
        let bounds = cx.debug_bounds(selector).expect("sidebar destination");
        cx.simulate_click(bounds.center(), Default::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command, Command::SidebarOpen { index, .. } if index == target
            )),
            "Audio Player intercepted {selector}"
        );
    }
    updates.send_replace(initial);
    cx.run_until_parked();
    draw(cx);
    assert!(cx.debug_bounds("browser-row-Artist A").is_some());
}
#[gpui::test]
fn pending_download_controls_dispatch_cancel_and_open_list(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.columns[0].rows = std::sync::Arc::new(vec![browser::Row {
        download_action: Some("Cancel Download"),
        download_id: Some("transfer".into()),
        download_status: Some("Downloading · 25%".into()),
        year: None,
        key: "track".into(),
        title: "Pending Song".into(),
        kind: browser::Kind::Track,
    }]);
    initial.status = "Downloading music…".into();
    initial.loading = Some(initial.status.clone());
    initial.download_notification = true;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..6 {
        draw(cx);
    }
    let cancel = cx
        .debug_bounds("download-cancel")
        .expect("pending row cancellation control");
    cx.simulate_click(cancel.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Select {
                intent: browser::Intent::ManageDownload,
                ..
            }
        ))
    );
    let status = cx
        .debug_bounds("utility-status")
        .expect("download notification");
    cx.simulate_click(status.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Palette(crate::app::state::PaletteCommandKind::Downloads(
                crate::app::downloads::Menu::Open
            ))
        ))
    );
}

#[gpui::test]
fn single_window_sidebar_presents_loaded_columns(cx: &mut TestAppContext) {
    sidebar_click_and_new_column_contents_are_presented(cx, false);
}
#[gpui::test]
fn multi_window_sidebar_presents_loaded_columns(cx: &mut TestAppContext) {
    sidebar_click_and_new_column_contents_are_presented(cx, true);
}
fn sidebar_click_and_new_column_contents_are_presented(cx: &mut TestAppContext, separate: bool) {
    cx.update(init);
    let mut initial = snapshot();
    initial.separate_player = separate;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    draw(cx);
    let library = cx
        .debug_bounds("library-workspace")
        .expect("library layout");
    assert!(
        library.size.height > cx.update(|window, _| window.viewport_size().height) - px(180.),
        "library must fill the space above the transport: {library:?}"
    );
    let artist = cx.debug_bounds("browser-row-Artist A").expect("artist row");
    assert!(artist.size.width > px(20.));
    let sidebar = cx.debug_bounds("sidebar-Tracks").unwrap();
    cx.simulate_click(sidebar.center(), Default::default());
    let mut selected = false;
    while let Ok(command) = commands.try_recv() {
        selected |= matches!(command, Command::SidebarOpen { index: 1, .. });
    }
    assert!(selected, "sidebar click was intercepted");
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    let track = crate::library::models::Track {
        rating_key: "track".into(),
        title: "Fixture Track".into(),
        ..Default::default()
    };
    state.artist_nav.columns = vec![BrowseColumn::new_with_tracks(
        "tracks",
        BrowseItem::from_tracks(std::slice::from_ref(&track)),
        vec![track],
    )];
    next.columns = browser::columns(&state);
    next.navigation_serial += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    draw(cx);
    draw(cx);
    let track = cx
        .debug_bounds("browser-row-Fixture Track")
        .expect("loaded track row");
    assert!(
        track.size.width > px(20.) && track.size.height > px(10.),
        "track bounds: {track:?}"
    );
    assert!(
        track.left() >= library.left() && track.right() <= library.right() + px(1.),
        "track not in viewport: {track:?}, library: {library:?}"
    );

    let playlist = cx.debug_bounds("sidebar-Fixture Playlist").unwrap();
    cx.simulate_click(playlist.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| { matches!(command, Command::SidebarOpen { index: 2, .. }) }),
        "playlist click was intercepted"
    );
    state.browse_category = crate::app::BrowseCategory::Playlists;
    state.playlist_nav.columns = vec![
        BrowseColumn::new("playlists", vec![]),
        state.artist_nav.columns.remove(0),
    ];
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.columns = browser::columns(&state);
    next.focused_column = 1;
    next.navigation_serial += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    draw(cx);
    draw(cx);
    let playlist_track = cx
        .debug_bounds("browser-row-Fixture Track")
        .expect("playlist contents");
    assert!(playlist_track.size.height > px(10.) && playlist_track.size.width > px(20.));
}

#[gpui::test]
fn newly_opened_album_columns_fit_titles_and_do_not_exceed_parent(cx: &mut TestAppContext) {
    cx.update(init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let initial = snapshot();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..4 {
        draw(cx);
    }
    let mut widths = Vec::new();
    for (title, covers, large) in [
        (
            "An Album with a Fairly Long Name for This Column",
            false,
            false,
        ),
        (
            "An Album with a Fairly Long Name for This Column",
            true,
            false,
        ),
        (
            "An Album with a Fairly Long Name for This Column",
            true,
            true,
        ),
        (
            "An Extremely Long Album Title That Cannot Possibly Fit Without Truncation Even If It Uses Half Of The Entire Window Width With Large Artwork And All The Other Controls",
            true,
            true,
        ),
    ] {
        let mut next = snapshot();
        let mut album = next.columns[0].clone();
        album.identity = "album-column".into();
        album.title = "albums — Artist A".into();
        album.rows = std::sync::Arc::new(vec![browser::Row {
            download_action: None,
            download_id: None,
            download_status: None,
            year: Some(2000),
            key: "album".into(),
            title: title.into(),
            kind: browser::Kind::Album,
        }]);
        album.revision = widths.len() as u64;
        album.artwork_visible = covers;
        album.album_grid = large;
        next.columns.push(album);
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..6 {
            draw(cx);
        }
        assert!(cx.debug_bounds("browser-row-Artist A").is_some());
        let (parent_width, album_width) = view.read_with(cx, |view, _| {
            let width = |index| {
                view.pane_presented
                    .get(&super::motion::PaneKey::Column(index))
                    .unwrap()
                    .size
                    .width
            };
            (width(0), width(1))
        });
        assert!(
            album_width <= parent_width + px(1.),
            "album {album_width:?} exceeds preceding column {parent_width:?}"
        );
        assert!(
            album_width > px(300.),
            "new column too narrow: {album_width:?}"
        );
        if covers {
            let cover = cx
                .debug_bounds("browser-artwork-1-0")
                .expect("album artwork");
            let label = cx
                .debug_bounds("browser-title-album")
                .expect("album caption");
            if large {
                assert!(
                    label.top() >= cover.bottom(),
                    "large artwork caption overlaps the image"
                );
                assert!(
                    (label.center().x - cover.center().x).abs() <= px(1.),
                    "large artwork caption must be centered below the cover"
                );
                assert!(label.size.height >= px(28.), "allow two caption lines");
            } else {
                assert!(
                    label.left() >= cover.right(),
                    "small artwork caption must be beside the image"
                );
            }
        }
        if !large {
            let label = cx.debug_bounds("browser-title-album").expect("album title");
            let text_width = cx.update(|window, cx| {
                let mut style = window.text_style();
                style.font_family = cx.theme().font_family.clone();
                window
                    .text_system()
                    .shape_line(title.into(), px(13.), &[style.to_run(title.len())], None)
                    .width
            });
            assert!(
                label.size.width >= text_width,
                "title clipped: text {text_width:?}, label {label:?}, column {album_width:?}"
            );
        }
        widths.push(album_width);
    }
    assert!(
        widths[3] > px(800.),
        "long title should use the available parent-width cap: {widths:?}"
    );
    let parent = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(0)]
    });
    let handle = gpui::point(parent.right(), parent.center().y);
    let drag_to = handle + gpui::point(px(80.), px(0.));
    cx.simulate_mouse_move(handle, None, Default::default());
    cx.simulate_mouse_down(handle, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        handle + gpui::point(px(20.), px(0.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    cx.simulate_mouse_move(drag_to, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_up(drag_to, gpui::MouseButton::Left, Default::default());
    for _ in 0..6 {
        draw(cx);
    }
    let resized = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(1)]
            .size
            .width
    });
    assert!(
        resized < widths[3] - px(30.),
        "divider did not resize: {resized:?}, before {:?}",
        widths[3]
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.navigation_serial += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..4 {
        draw(cx);
    }
    let retained = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(1)]
            .size
            .width
    });
    assert!(
        (retained - resized).abs() < px(1.),
        "navigation reset the dragged width"
    );
}

#[gpui::test]
fn equal_columns_keep_a_readable_minimum_and_scroll_horizontally(cx: &mut TestAppContext) {
    cx.update(init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let initial = snapshot();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    let mut grouped = snapshot();
    grouped.columns[0].rows = std::sync::Arc::new(vec![browser::Row {
        download_action: None,
        download_id: None,
        download_status: None,
        year: None,
        key: "group".into(),
        title: "A Playlist Album with a Long Title and Artwork".into(),
        kind: browser::Kind::Album,
    }]);
    grouped.columns[0].revision += 1;
    grouped.columns[0].artwork_visible = true;
    grouped.columns[0].album_grid = true;
    let mut tracks = grouped.columns[0].clone();
    tracks.identity = "playlist-album-tracks".into();
    tracks.artwork_visible = true;
    tracks.album_grid = false;
    tracks.rows = std::sync::Arc::new(vec![browser::Row {
        download_action: None,
        download_id: None,
        download_status: None,
        year: None,
        key: "fixture-song".into(),
        title: "A Track Title That Must Not Be Squeezed Into a Tiny Column".into(),
        kind: browser::Kind::Track,
    }]);
    tracks.tracks = vec![crate::library::models::Track {
        rating_key: "fixture-song".into(),
        title: tracks.rows[0].title.clone(),
        grandparent_title: Some("An Artist with a Long Name".into()),
        parent_title: Some("The Album Name".into()),
        ..Default::default()
    }]
    .into();
    grouped.columns.push(tracks.clone());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.apply_snapshot(grouped.clone(), window, cx)
        })
    });
    for _ in 0..6 {
        draw(cx);
    }
    let label = cx.debug_bounds("browser-title-fixture-song").unwrap();
    assert!(
        label.size.width > px(350.),
        "track title squeezed: {label:?}"
    );
    let mut deeper = grouped.clone();
    tracks.identity = "child-track-column".into();
    tracks.rows = std::sync::Arc::new(vec![browser::Row {
        download_action: None,
        download_id: None,
        download_status: None,
        year: None,
        key: "child-song".into(),
        title: "Another Track with a Long Enough Title to Need Space".into(),
        kind: browser::Kind::Track,
    }]);
    deeper.columns.push(tracks);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.apply_snapshot(deeper.clone(), window, cx)
        })
    });
    for _ in 0..6 {
        draw(cx);
    }
    let widths = view.read_with(cx, |view, _| {
        (0..3)
            .map(|index| {
                view.pane_presented[&super::motion::PaneKey::Column(index)]
                    .size
                    .width
            })
            .collect::<Vec<_>>()
    });
    assert!(
        widths
            .iter()
            .all(|width| *width >= px(super::column_layout::MIN_COLUMN)),
        "ancestors did not share space: {widths:?}"
    );
    assert!(widths.windows(2).all(|pair| pair[0] + px(1.) >= pair[1]));
    for index in 3..6 {
        let mut child = deeper.columns.last().unwrap().clone();
        child.identity = format!("deep-folder-column-{index}");
        deeper.columns.push(child);
    }
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(deeper, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    let widths = view.read_with(cx, |view, _| {
        (0..6)
            .map(|index| {
                view.pane_presented[&super::motion::PaneKey::Column(index)]
                    .size
                    .width
            })
            .collect::<Vec<_>>()
    });
    assert!(
        widths
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() < px(1.)),
        "equal-width mode: {widths:?}"
    );
    let strip = cx.debug_bounds("column-strip").unwrap();
    let newest = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(5)]
    });
    assert!(
        newest.right() <= strip.right() + px(1.),
        "new column must be fully revealed: {newest:?}, viewport {strip:?}"
    );
    assert!(cx.debug_bounds("column-horizontal-scrollbar").is_some());
    cx.simulate_resize(gpui::size(px(900.), px(900.)));
    for _ in 0..6 {
        draw(cx);
    }
    let resized = view.read_with(cx, |view, _| {
        (0..6)
            .map(|index| {
                view.pane_presented[&super::motion::PaneKey::Column(index)]
                    .size
                    .width
            })
            .collect::<Vec<_>>()
    });
    assert!(
        resized
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() < px(1.)),
        "resized equal-width mode: {resized:?}"
    );
    let viewport = cx.debug_bounds("library-workspace").unwrap();
    let last = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(5)]
    });
    assert!(
        resized
            .iter()
            .all(|width| *width >= px(super::column_layout::MIN_COLUMN))
    );
    let total: gpui::Pixels = resized
        .iter()
        .copied()
        .fold(px(0.), |sum, width| sum + width);
    assert!(
        total > viewport.size.width,
        "deep columns need horizontal overflow"
    );
    let gutter = cx.debug_bounds("column-horizontal-scrollbar").unwrap();
    let strip = cx.debug_bounds("column-strip").unwrap();
    assert!(
        gutter.top() >= strip.bottom(),
        "scrollbar must be outside scrolling content"
    );
    assert!(gutter.size.height >= px(12.));
    let rightmost = view.read_with(cx, |view, _| -view.column_scroll.max_offset().x);
    assert_eq!(
        view.read_with(cx, |view, _| view.column_scroll.offset().x),
        rightmost
    );
    let before = view.read_with(cx, |view, _| view.column_scroll.offset().x);
    cx.simulate_event(gpui::ScrollEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(150.), px(0.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    draw(cx);
    let after = view.read_with(cx, |view, _| view.column_scroll.offset().x);
    assert_ne!(
        before, after,
        "horizontal scrolling must move the column strip: {last:?}"
    );

    let mut replacement = view.read_with(cx, |view, _| view.snapshot.clone());
    replacement.columns.last_mut().unwrap().identity = "replacement-destination".into();
    view.update(cx, |view, _| {
        view.column_scroll.set_offset(gpui::point(px(0.), px(0.)))
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.apply_snapshot(replacement, window, cx))
    });
    for _ in 0..6 {
        draw(cx);
    }
    let strip = cx.debug_bounds("column-strip").unwrap();
    let destination = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(5)]
    });
    assert!(
        destination.right() <= strip.right() + px(1.),
        "replacement column must also be revealed: {destination:?}"
    );

    let gutter = cx.debug_bounds("column-horizontal-scrollbar").unwrap();
    let before = view.read_with(cx, |view, _| view.column_scroll.offset().x);
    cx.simulate_click(
        gpui::point(gutter.left() + px(8.), gutter.center().y),
        Default::default(),
    );
    draw(cx);
    assert!(
        view.read_with(cx, |view, _| view.column_scroll.offset().x) > before,
        "persistent scrollbar must accept track clicks"
    );
    let mut inspected = view.read_with(cx, |view, _| view.snapshot.clone());
    inspected.preview = Some(catalog::Preview {
        title: "First Details".into(),
        ..Default::default()
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.apply_snapshot(inspected.clone(), window, cx)
        })
    });
    for _ in 0..6 {
        draw(cx);
    }
    view.update(cx, |view, _| {
        view.column_scroll.set_offset(gpui::point(px(0.), px(0.)))
    });
    inspected.preview.as_mut().unwrap().title = "Second Details".into();
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(inspected, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert_eq!(
        view.read_with(cx, |view, _| view.column_scroll.offset().x),
        view.read_with(cx, |view, _| -view.column_scroll.max_offset().x),
        "changing an existing details pane must fully reveal it"
    );
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(grouped, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    let library = cx.debug_bounds("library-workspace").unwrap();
    let right = view.read_with(cx, |view, _| {
        view.pane_presented[&super::motion::PaneKey::Column(1)].right()
    });
    assert!(
        (right - library.right()).abs() < px(1.),
        "shorter paths must fill the browser"
    );
}

#[gpui::test]
fn multi_window_settings_opens_a_native_window_without_covering_library(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.separate_player = true;
    initial.preferences = super::preferences::project(&AppState::new());
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let windows = WindowRegistry::default();
    let registry = windows.clone();
    let desktop_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = desktop_slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let desktop = cx.new(|cx| {
            let mut desktop = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                registry,
                window,
                cx,
            );
            desktop.snapshot = initial;
            desktop
        });
        slot.replace(Some(desktop.clone()));
        gpui_kit::base::Root::new(desktop, window, cx)
    });
    windows.borrow_mut()[WindowRole::Library.index()] =
        Some(cx.update(|window, _| window.window_handle()));
    let desktop = desktop_slot.borrow().clone().unwrap();
    for _ in 0..4 {
        draw(cx);
    }
    cx.update(|_, cx| desktop.update(cx, |desktop, cx| desktop.open_settings(cx)));
    cx.run_until_parked();
    for _ in 0..4 {
        draw(cx);
    }
    assert!(
        cx.debug_bounds("settings-content").is_none(),
        "Settings belongs to its own native window"
    );
    let handle = windows.borrow()[super::SETTINGS_WINDOW].expect("native Settings window");
    assert_ne!(
        handle.window_id(),
        cx.update(|window, _| window.window_handle().window_id())
    );
    assert!(windows.settings_editor.borrow().is_some());
    cx.update(|_, cx| {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    });
    for _ in 0..2 {
        let mut transferred = desktop.read_with(cx, |desktop, _| desktop.snapshot.clone());
        transferred.presentation = crate::app::windowed_presentation::Presentation::windowed();
        let text = transferred
            .presentation
            .request(crate::app::launch::WindowedMode::Text)
            .unwrap();
        assert!(transferred.presentation.commit(text));
        desktop.update(cx, |desktop, _| {
            desktop.snapshot.presentation = transferred.presentation.clone()
        });
        let gui = transferred
            .presentation
            .request(crate::app::launch::WindowedMode::Gui)
            .unwrap();
        assert!(transferred.presentation.commit(gui));
        transferred.settings_request += 1;
        transferred.library_window_request += 1;
        cx.update(|window, cx| {
            desktop.update(cx, |desktop, cx| {
                desktop.apply_snapshot(transferred, window, cx)
            })
        });
        for _ in 0..4 {
            draw(cx);
        }
        let settings = windows.borrow()[super::SETTINGS_WINDOW].unwrap();
        assert_eq!(
            cx.update(|_, cx| cx.active_window())
                .map(|handle| handle.window_id()),
            Some(settings.window_id()),
            "Transferred Settings must be in front of the Library window"
        );
    }
    let settings = windows.borrow()[super::SETTINGS_WINDOW].unwrap();
    cx.update(|_, cx| {
        let _ = settings.update(cx, |_, window, _| window.remove_window());
    });
}

#[gpui::test]
fn header_close_icon_targets_the_clicked_column_in_equal_width_mode(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    for index in 1..3 {
        let mut column = initial.columns[0].clone();
        column.identity = format!("column-{index}");
        column.title = format!("Column {index}");
        initial.columns.push(column);
    }
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    for _ in 0..4 {
        draw(cx);
    }
    for index in ["close-column-0", "close-column-1", "close-column-2"] {
        let icon = cx.debug_bounds(index).expect("column close icon");
        assert!(icon.size.width > px(10.) && icon.size.width <= px(24.));
    }
    let close = cx.debug_bounds("close-column-1").unwrap().center();
    cx.simulate_click(close, Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
        Command::CloseColumn { column: 1, identity, .. } if identity == "column-1"))
    );
}

#[gpui::test]
fn detail_pane_has_a_compact_scoped_close_button(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.generation = 7;
    initial.navigation_serial = 23;
    initial.preview = Some(catalog::Preview {
        title: "Track details".into(),
        ..Default::default()
    });
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    for _ in 0..4 {
        draw(cx);
    }
    let close = cx
        .debug_bounds("close-preview")
        .expect("details close icon");
    assert!(close.size.width > px(10.) && close.size.width <= px(24.));
    cx.simulate_click(close.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::ClosePreview {
                generation: 7,
                navigation_serial: 23
            }
        ))
    );
}

#[gpui::test]
fn return_opens_and_alphabet_rail_uses_available_height(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.bind_keys([gpui::KeyBinding::new(
            "enter",
            super::Activate,
            Some("Textamp"),
        )]);
    });
    let mut initial = snapshot();
    initial.columns[0].alphabet_ascending = Some(true);
    let identity = initial.columns[0].identity.clone();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view.letter_shortcuts = Some((
            identity,
            std::time::Instant::now() + std::time::Duration::from_secs(60),
        ));
        view.focus.focus(window, cx);
        view
    });
    for _ in 0..5 {
        draw(cx);
    }
    let first = cx.debug_bounds("jump-0-%").unwrap();
    let last = cx.debug_bounds("jump-0-Z").unwrap();
    let rail = cx.debug_bounds("alphabet-rail-0").unwrap();
    assert!(first.top() >= rail.top() && last.bottom() <= rail.bottom());
    assert!(
        last.bottom() - first.top() > rail.size.height * 0.9,
        "letters must use the rail: {first:?} {last:?} {rail:?}"
    );
    while commands.try_recv().is_ok() {}
    cx.simulate_click(last.center(), Default::default());
    let clicked: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(matches!(
        clicked.as_slice(),
        [Command::JumpLetter {
            column: 0,
            letter: 'Z',
            ..
        }]
    ));
    cx.simulate_keystrokes("enter");
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(|c| matches!(
        c,
        Command::Navigate {
            column_delta: 1,
            row_delta: 0,
            ..
        }
    )));
    assert!(!sent.iter().any(|c| matches!(c, Command::Activate { .. })));
    let viewport = cx.debug_bounds("column-viewport-0").unwrap();
    view.update(cx, |view, cx| {
        view.letter_shortcuts = None;
        cx.notify();
    });
    draw(cx);
    assert_eq!(cx.debug_bounds("column-viewport-0"), Some(viewport));
}

#[gpui::test]
fn artwork_mini_returns_to_library_and_now_playing(cx: &mut TestAppContext) {
    cx.update(init);
    for sheet in [false, true] {
        let mut initial = snapshot();
        initial.player.active = sheet;
        let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        cx.simulate_resize(gpui::size(px(1200.), px(800.)));
        for _ in 0..5 {
            draw(cx);
        }
        let cover = cx
            .debug_bounds(if sheet {
                "transport-artwork"
            } else {
                "transport-cover-slot"
            })
            .unwrap();
        cx.simulate_click(cover.center(), Default::default());
        draw(cx);
        let art = cx.debug_bounds("mini-artwork-return").unwrap();
        cx.simulate_click(art.center(), Default::default());
        for _ in 0..4 {
            draw(cx);
        }
        assert!(cx.debug_bounds("mini-artwork-return").is_none());
        assert_eq!(cx.debug_bounds("collapse-player-sheet").is_some(), sheet);
        assert!(
            !std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                c,
                Command::Palette(
                    crate::app::state::PaletteCommandKind::GotoNowPlaying
                        | crate::app::state::PaletteCommandKind::GotoLibrary
                )
            ))
        );
    }
}

#[gpui::test]
fn mini_player_artwork_fills_the_width_of_a_tall_window(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::NowPlaying,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(500.), px(760.)));
    for _ in 0..5 {
        draw(cx);
    }
    let art = cx.debug_bounds("mini-artwork").unwrap();
    assert!((art.size.width - art.size.height).abs() < px(1.));
    assert!(
        art.size.width >= px(480.),
        "artwork should be nearly edge-to-edge: {art:?}"
    );
}

#[gpui::test]
fn apple_music_shortcut_invokes_the_native_menu_command(cx: &mut TestAppContext) {
    let handled = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = handled.clone();
    cx.update(move |cx| {
        init(cx);
        super::keyboard::bind_navigation_shortcuts(cx);
        cx.on_action(move |action: &super::menu_catalog::Invoke, _| {
            observed.set(action.key == "SearchAppleMusic");
        });
    });
    let initial = snapshot();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view.focus.focus(window, cx);
        view
    });
    draw(cx);
    cx.simulate_keystrokes("cmd-shift-a");
    assert!(
        handled.get(),
        "Apple Music shortcut did not reach the menu command"
    );
}

#[gpui::test]
fn long_album_link_wraps_within_the_track_detail_value_column(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.preview = Some(catalog::Preview {
        title: "Flip The Switch".into(),
        track: Some(Default::default()),
        rows: vec![(
            "Album".into(),
            "Bridges To Babylon [1997] (Expanded Edition With Additional Tracks)".into(),
        )],
        ..Default::default()
    });
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(820.), px(650.)));
    draw(cx);
    draw(cx);
    let pane = cx.debug_bounds("details-pane").unwrap();
    let button = cx.debug_bounds("detail-link-0").unwrap();
    let value = cx.debug_bounds("detail-value-0").unwrap();
    assert!(value.left() >= button.left() && value.right() <= pane.right());
    assert!(
        value.size.height > px(22.) && button.size.height >= value.size.height,
        "{button:?} {value:?}"
    );
}

#[gpui::test]
fn equal_width_details_share_space(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.preview = Some(catalog::Preview {
        track_artwork: true,
        title: "A long track title that wraps across several lines in a narrow detail column"
            .into(),
        rows: (0..20)
            .map(|index| (format!("Field {index}"), "Metadata".into()))
            .collect(),
        ..Default::default()
    });
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let (_view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    draw(cx);
    let column = cx.debug_bounds("browser-column-0").expect("browser column");
    let details = cx.debug_bounds("details-pane").expect("details");
    assert!(
        (column.size.width - details.size.width).abs() < px(2.),
        "column {column:?}; details {details:?}"
    );
    assert!(cx.debug_bounds("library-back").is_none());
    let artwork = cx.debug_bounds("details-artwork").expect("details artwork");
    assert!(artwork.size.width > px(80.));
    assert!(
        (artwork.size.width - artwork.size.height).abs() < px(2.),
        "artwork must remain square: {artwork:?}"
    );

    assert!(cx.debug_bounds("sidebar-search").is_some());
    assert!(cx.debug_bounds("sidebar-palette").is_some());
    let handle = gpui::point(column.right(), column.center().y);
    let destination = handle + gpui::point(px(70.), px(0.));
    cx.simulate_mouse_move(handle, None, Default::default());
    cx.simulate_mouse_down(handle, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        handle + gpui::point(px(20.), px(0.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    cx.simulate_mouse_move(destination, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_up(destination, gpui::MouseButton::Left, Default::default());
    for _ in 0..6 {
        draw(cx);
    }
    let adjusted = cx.debug_bounds("browser-column-0").unwrap();
    let adjusted_details = cx.debug_bounds("details-pane").unwrap();
    assert!(adjusted.size.width > column.size.width + px(30.));
    assert!(adjusted_details.size.width < details.size.width - px(30.));
    for _ in 0..4 {
        draw(cx);
    }
    assert_eq!(cx.debug_bounds("browser-column-0").unwrap(), adjusted);
}

#[gpui::test]
fn queue_tools_menu_restores_queue_without_dispatching_navigation_undo(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.queue_open = true;
    initial
        .queue_tools
        .iter_mut()
        .find(|tool| tool.tool == crate::app::presentation::NpSidebarButton::Undo)
        .unwrap()
        .disabled_reason = None;
    initial
        .queue_tools
        .retain(|tool| tool.tool == crate::app::presentation::NpSidebarButton::Undo);
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    draw(cx);
    draw(cx);
    let undo = cx
        .debug_bounds("queue-tools")
        .expect("queue tools dropdown");
    while commands.try_recv().is_ok() {}
    cx.simulate_click(undo.center(), Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(|command| matches!(
        command,
        Command::QueueTool {
            tool: crate::app::presentation::NpSidebarButton::Undo,
            ..
        }
    )));
    assert!(
        !sent
            .iter()
            .any(|command| matches!(command, Command::Palette(_)))
    );
    view.update(cx, |view, cx| {
        view.snapshot
            .queue_tools
            .iter_mut()
            .find(|tool| tool.tool == crate::app::presentation::NpSidebarButton::Undo)
            .unwrap()
            .disabled_reason = Some("No queue edit to undo");
        cx.notify();
    });
    draw(cx);
    while commands.try_recv().is_ok() {}
    cx.simulate_click(undo.center(), Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::QueueTool {
                tool: crate::app::presentation::NpSidebarButton::Undo,
                ..
            }
        ))
    );
}

#[gpui::test]
fn queue_drag_from_sidebar_preserves_row_identity(cx: &mut TestAppContext) {
    queue_drag(cx, WindowRole::Library);
}

#[gpui::test]
fn queue_drag_from_independent_window_preserves_row_identity(cx: &mut TestAppContext) {
    queue_drag(cx, WindowRole::Queue);
}

fn queue_drag(cx: &mut TestAppContext, role: WindowRole) {
    cx.update(init);
    let mut state = AppState::new();
    state.queue.tracks = ["First", "Second", "Third"]
        .map(|title| crate::library::models::Track {
            rating_key: title.into(),
            title: title.into(),
            ..Default::default()
        })
        .into();
    let mut initial = snapshot();
    initial.queue_open = true;
    initial.player.active = true;
    initial.separate_player = role == WindowRole::Queue;
    initial.playback_queue = Some(browser::queue(&state));
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            role,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    draw(cx);
    draw(cx);
    let from = cx.debug_bounds("browser-row-First").unwrap().center();
    let third = cx.debug_bounds("browser-row-Third").unwrap();
    let to = gpui::point(third.center().x, third.bottom() - px(8.));
    while commands.try_recv().is_ok() {}
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
    cx.simulate_mouse_move(
        from + gpui::point(px(0.), px(12.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    draw(cx);
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Default::default());
    draw(cx);
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
    let commands: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    let drop = commands
        .iter()
        .find_map(|command| match command {
            Command::QueueDrop { from, to, after } => Some((from, to, *after)),
            _ => None,
        })
        .expect("queue drop command");
    assert_eq!((drop.0.row, drop.1.row, drop.2), (0, 2, true));
    assert!(browser::valid(&state, drop.0));
    assert!(browser::valid(&state, drop.1));
}

#[gpui::test]
fn album_grid_keys_distinguish_horizontal_movement_from_opening(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.bind_keys([
            gpui::KeyBinding::new("right", super::MoveRight, Some("Textamp")),
            gpui::KeyBinding::new("left", super::MoveLeft, Some("Textamp")),
            gpui::KeyBinding::new("enter", super::Activate, Some("Textamp")),
        ]);
    });
    let mut initial = snapshot();
    initial.columns[0].album_grid = true;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view.focus.focus(window, cx);
            view
        });
        gpui_kit::base::Root::new(view, window, cx)
    });
    draw(cx);
    while commands.try_recv().is_ok() {}
    cx.simulate_keystrokes("right left enter");
    let keys = std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Navigate {
                column_delta,
                row_delta: 0,
                ..
            } => Some(column_delta),
            Command::Activate { following: false } => Some(0),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(keys, vec![1, -1, 0]);
}

#[gpui::test]
fn artist_album_list_opens_its_own_biography(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.columns[0].artist_header = Some((
        "artist".into(),
        "Artist".into(),
        "A long biography. ".repeat(100),
    ));
    initial.columns[0].album_grid = true;
    initial.columns[0].artwork_visible = true;
    initial.columns[0].rows = std::sync::Arc::new(
        (0..3)
            .map(|i| browser::Row {
                key: format!("album-{i}"),
                title: if i == 1 {
                    "An unusually long album title ".repeat(4)
                } else {
                    format!("Album {i}")
                },
                kind: browser::Kind::Album,
                download_action: None,
                download_id: None,
                download_status: None,
                year: None,
            })
            .collect(),
    );
    initial.columns[0].similar_artists = (0..12)
        .map(|index| crate::library::models::Artist {
            rating_key: format!("related-{index}"),
            title: format!("Related Artist {index}"),
            ..Default::default()
        })
        .collect();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    draw(cx);
    let page = cx.debug_bounds("artist-page-0").unwrap();
    let portrait = cx.debug_bounds("artist-portrait-0").unwrap();
    assert!((portrait.center().x - page.center().x).abs() <= px(1.));
    let excerpt = cx.debug_bounds("artist-bio-excerpt-0").unwrap();
    let button = cx.debug_bounds("artist-header-bio-0").unwrap();
    assert!(excerpt.size.height <= px(54.) && excerpt.bottom() <= button.top());
    let [first, second, third] = [
        "browser-artwork-0-0",
        "browser-artwork-0-1",
        "browser-artwork-0-2",
    ]
    .map(|key| cx.debug_bounds(key).unwrap());
    assert_eq!(first.size, gpui::size(px(72.), px(72.)));
    assert_eq!(first.size, second.size);
    assert_eq!(first.top(), second.top());
    assert_eq!(
        first.top(),
        third.top(),
        "the wide column fits a third cover"
    );
    assert!(second.left() >= first.right() && third.left() >= second.right());
    cx.simulate_resize(gpui::size(px(800.), px(620.)));
    draw(cx);
    let page = cx.debug_bounds("artist-page-0").unwrap();
    let first = cx.debug_bounds("browser-artwork-0-0").unwrap();
    let third = cx.debug_bounds("browser-artwork-0-2").unwrap();
    if page.size.width < px(408.) {
        assert_eq!(first.left(), third.left());
        assert!(third.top() >= first.bottom());
    }
    let button = cx.debug_bounds("artist-header-bio-0").unwrap();
    cx.simulate_click(button.center(), Default::default());
    assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|command| matches!(command,
        Command::GuardedActions {actions,..} if matches!(actions.as_slice(),
            [crate::app::Action::Search(crate::app::action::SearchAction::ShowArtistBio {artist_key,..})] if artist_key=="artist"))));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let column = &mut view.snapshot.columns[0];
            column.artist_header.as_mut().unwrap().2.clear();
            column.bio_search = Some(Default::default());
            cx.notify();
        })
    });
    draw(cx);
    for element in ["artist-header-bio-0", "artist-portrait-0"] {
        let bounds = cx.debug_bounds(element).unwrap();
        assert!((bounds.center().x - page.center().x).abs() <= px(1.));
    }
    let first_before = cx.debug_bounds("browser-artwork-0-0").unwrap();
    let related_before = cx.debug_bounds("related-artist-cover-0").unwrap();
    assert_eq!(first_before.size, related_before.size);
    assert_eq!(first_before.left(), related_before.left());
    cx.simulate_event(gpui::ScrollEvent {
        position: first_before.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-140.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    draw(cx);
    let album_shift = first_before.top() - cx.debug_bounds("browser-artwork-0-0").unwrap().top();
    let artist_shift =
        related_before.top() - cx.debug_bounds("related-artist-cover-0").unwrap().top();
    assert!(album_shift > px(0.));
    assert_eq!(album_shift, artist_shift);
}

#[gpui::test]
fn track_header_icons_route_option_clicks_and_collection_headers_ignore_option(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let tracks: Vec<_> = (0..3)
        .map(|index| crate::library::models::Track {
            rating_key: format!("track-{index}"),
            title: format!("Track {index}"),
            ..Default::default()
        })
        .collect();
    let mut initial = snapshot();
    initial.preview = Some(catalog::Preview {
        title: tracks[1].title.clone(),
        controls: catalog::compact_controls(crate::app::playback_controls::track_controls(
            &tracks[1],
            Some(&tracks[1..]),
        )),
        ..Default::default()
    });
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    draw(cx);
    draw(cx);
    for option in [false, true] {
        for index in 0..2 {
            let button = cx
                .debug_bounds(["details-control-0", "details-control-1"][index])
                .expect("header action");
            while commands.try_recv().is_ok() {}
            cx.simulate_click(
                button.center(),
                gpui::Modifiers {
                    alt: option,
                    ..Default::default()
                },
            );
            let actions = std::iter::from_fn(|| commands.try_recv().ok())
                .find_map(|command| match command {
                    Command::GuardedActions { actions, .. } => Some(actions),
                    _ => None,
                })
                .expect("queue action");
            use crate::app::{
                Action,
                action::{QueueAction, QueueLoadIntent},
            };
            match (index, option, actions.as_slice()) {
                (0, false, [Action::Queue(QueueAction::PlayTrack(track))]) => {
                    assert_eq!(track.rating_key, "track-1")
                }
                (
                    1,
                    false,
                    [
                        Action::Queue(QueueAction::TracksLoaded {
                            intent: QueueLoadIntent::Append { .. },
                            result: Ok(tracks),
                        }),
                    ],
                ) => {
                    assert_eq!(tracks.len(), 1);
                    assert_eq!(tracks[0].rating_key, "track-1");
                }
                (0, true, [Action::Queue(QueueAction::EnqueueTracksNext(tracks))]) => {
                    assert_eq!(tracks.len(), 1);
                }
                (
                    1,
                    true,
                    [
                        Action::Queue(QueueAction::TracksLoaded {
                            intent: QueueLoadIntent::Append { .. },
                            result: Ok(tracks),
                        }),
                    ],
                ) => {
                    assert_eq!(
                        tracks
                            .iter()
                            .map(|track| track.rating_key.as_str())
                            .collect::<Vec<_>>(),
                        ["track-1"]
                    );
                }
                other => panic!("wrong header action: {other:?}"),
            }
        }
    }
    assert!(cx.debug_bounds("details-control-2").is_none());
    let album = crate::app::state::PlayAllRow::Album {
        rating_key: "album".into(),
        title: "Album".into(),
    };
    let playlist = crate::app::state::PlayAllRow::Playlist {
        rating_key: "playlist".into(),
        title: "Playlist".into(),
    };
    for row in [album, playlist] {
        let controls = catalog::collection_controls(&row, &tracks);
        let is_album = matches!(row, crate::app::state::PlayAllRow::Album { .. });
        let count = if is_album { 3 } else { 2 };
        assert_eq!(controls.len(), count);
        assert!(controls.iter().all(|control| control.alternate.is_none()));
        view.update(cx, |view, cx| {
            view.snapshot.preview = None;
            view.snapshot.columns[0].album_page = is_album.then(|| {
                let mut info = crate::app::sources::navidrome::album_notes::AlbumInfo::default();
                info.review
                    .push(crate::app::sources::navidrome::album_notes::Passage {
                        text: "Review".into(),
                        italic: false,
                    });
                (
                    "album".into(),
                    catalog::Preview {
                        kind: catalog::PreviewKind::Album,
                        title: "Album".into(),
                        subtitle: "Artist".into(),
                        rows: vec![("Year".into(), "2012".into())],
                        artist_bio: Some(catalog::Control::new(
                            "Artist Biography",
                            crate::app::action::SearchAction::ShowArtistBio {
                                artist_key: "artist".into(),
                                artist_name: "Artist".into(),
                            },
                        )),
                        album_info: Some(info),
                        ..Default::default()
                    },
                )
            });
            view.snapshot.columns[0].header_controls = controls;
            cx.notify();
        });
        draw(cx);
        if is_album {
            let play = cx.debug_bounds("column-control-0-0").unwrap();
            let next = cx.debug_bounds("column-control-0-1").unwrap();
            let append = cx.debug_bounds("column-control-0-2").unwrap();
            let bio = cx.debug_bounds("album-bio-0").unwrap();
            let info = cx.debug_bounds("album-notes-0").unwrap();
            let heading = cx.debug_bounds("details-heading").unwrap();
            let metadata = cx.debug_bounds("details-metadata").unwrap();
            assert!(heading.bottom() <= play.top() && play.bottom() <= metadata.top());
            assert_eq!(bio.center().y, info.center().y);
            assert!(bio.center().x < play.center().x && play.center().x < info.center().x);
            cx.simulate_click(bio.center(), Default::default());
            assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|command| matches!(command,
                Command::GuardedActions {actions,..} if matches!(actions.as_slice(),
                    [crate::app::Action::Search(crate::app::action::SearchAction::ShowArtistBio {artist_key,..})] if artist_key=="artist"))));
            assert_eq!(next.center().y, append.center().y);
            assert!(next.center().y < play.center().y);
        }
        for option in [false, true] {
            for index in 0..count {
                while commands.try_recv().is_ok() {}
                let button = cx
                    .debug_bounds(
                        [
                            "column-control-0-0",
                            "column-control-0-1",
                            "column-control-0-2",
                        ][index],
                    )
                    .expect("collection header control");
                cx.simulate_click(
                    button.center(),
                    gpui::Modifiers {
                        alt: option,
                        ..Default::default()
                    },
                );
                assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
                    Command::ColumnControl {control, option: alternate, ..} if control==index && alternate==option
                )));
            }
        }
    }
}

#[gpui::test]
fn palette_backdrop_click_on_toolbar_dismisses_without_reopening(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut state = AppState::new();
    state.palette.open = true;
    let model = super::surface::project(&state).expect("palette surface");
    let mut initial = snapshot();
    initial.surface = Some(model.clone());
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let model_engine = engine.clone();
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let saved = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let desktop = cx.new(|cx| {
            let mut desktop = Desktop::new(
                engine,
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            desktop.snapshot = initial;
            desktop
        });
        saved.replace(Some(desktop.clone()));
        gpui_kit::base::Root::new(desktop, window, cx)
    });
    let desktop = slot.borrow().clone().unwrap();
    draw(cx);
    draw(cx);
    let toggle = cx
        .debug_bounds("sidebar-palette")
        .expect("palette toolbar icon");
    cx.update(|window, cx| {
        super::surface_view::open_overlay(
            desktop.downgrade(),
            model,
            model_engine.commands.clone(),
            model_engine.snapshot.clone(),
            window,
            cx,
        )
    });
    draw(cx);
    draw(cx);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    while commands.try_recv().is_ok() {}
    cx.simulate_click(toggle.center(), Default::default());
    draw(cx);
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        sent.iter()
            .any(|command| matches!(command, Command::CloseSurface(_)))
    );
    assert!(
        !sent
            .iter()
            .any(|command| matches!(command, Command::OpenPalette { .. }))
    );
}

#[gpui::test]
fn queue_omits_radio_button_and_clear_still_stops_the_station(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut state = AppState::new();
    state.set_playback_mode(crate::app::state::PlaybackMode::Radio);
    state.radio.active_station = Some(crate::app::state::ActiveStation {
        source: crate::library::models::RadioSource::Station("station".into()),
        title: "Station".into(),
    });
    let mut initial = snapshot();
    initial.player =
        super::player_model::project(&state, &mut super::player_model::Projection::default());
    initial.player.active = true;
    initial.queue_open = true;
    assert!(state.radio.active_station.is_some());
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..6 {
        draw(cx);
    }
    assert!(cx.debug_bounds("queue-radio").is_none());
    let radio = super::surface::radio_surface(&state);
    assert!(radio.buttons.iter().any(|item| item.label == "Manual Queue" && matches!(&item.operation,
        super::surface::Operation::Actions(actions) if matches!(actions.as_slice(), [crate::app::Action::Radio(crate::app::action::RadioAction::StopStation)])
    )));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    crate::app::handlers::dispatch_queue::dispatch(
        &tx,
        crate::app::action::QueueAction::ClearQueue,
        &mut state,
        &mut crate::audio::AudioPlayer::new_without_audio(),
    )
    .unwrap();
    assert!(state.radio.active_station.is_none());
    assert!(
        !super::surface::radio_surface(&state)
            .buttons
            .iter()
            .any(|button| button.label == "Manual Queue")
    );
}

#[gpui::test]
fn single_window_queue_spans_now_playing_height(cx: &mut TestAppContext) {
    queue_spans_now_playing_height(cx, false);
}

#[gpui::test]
fn independent_window_queue_spans_now_playing_height(cx: &mut TestAppContext) {
    queue_spans_now_playing_height(cx, true);
}

fn queue_spans_now_playing_height(cx: &mut TestAppContext, separate: bool) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    initial.player.active = !separate;
    initial.queue_open = true;
    initial.separate_player = separate;
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            if separate {
                WindowRole::Queue
            } else {
                WindowRole::Library
            },
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for (width, height) in [(1100., 800.), (900., 700.)] {
        cx.simulate_resize(gpui::size(px(width), px(height)));
        for _ in 0..6 {
            draw(cx);
        }
        let queue = cx.debug_bounds("play-queue").expect("independent queue");

        let tools = cx.debug_bounds("queue-tools").unwrap();
        let mode = cx.debug_bounds("queue-mode").unwrap();
        assert!(mode.bottom() <= tools.top());
        if separate {
            assert!(queue.size.height > px(height - 140.), "{queue:?}");
            assert!(tools.top() > queue.center().y);
            assert!(queue.size.width >= px(width - super::motion::SIDEBAR_RAIL_WIDTH - 2.));
            assert!(cx.debug_bounds("transport-bar").is_none());
        } else {
            let bar = cx.debug_bounds("transport-bar").unwrap();
            assert!((bar.size.width - px(width)).abs() <= px(2.));
            assert!(tools.bottom() <= bar.top());
            assert!((queue.bottom() - bar.top()).abs() < px(2.));
            let main = cx.debug_bounds("workspace-content").unwrap();
            assert!((queue.top() - main.top()).abs() < px(2.));
            assert!((queue.bottom() - main.bottom()).abs() < px(2.));
            assert!(cx.debug_bounds("collapse-player-sheet").is_some());
            let handle = gpui::point(queue.left(), queue.center().y);
            let delta = if queue.size.width < px(310.) {
                70.
            } else {
                -70.
            };
            let destination = handle - gpui::point(px(delta), px(0.));
            cx.simulate_mouse_move(handle, None, Default::default());
            cx.simulate_mouse_down(handle, gpui::MouseButton::Left, Default::default());
            cx.simulate_mouse_move(
                handle - gpui::point(px(20.), px(0.)),
                gpui::MouseButton::Left,
                Default::default(),
            );
            cx.simulate_mouse_move(destination, gpui::MouseButton::Left, Default::default());
            cx.simulate_mouse_up(destination, gpui::MouseButton::Left, Default::default());
            for _ in 0..6 {
                draw(cx);
            }
            assert!(
                (cx.debug_bounds("play-queue").unwrap().size.width - queue.size.width).abs()
                    > px(30.)
            );
        }
    }
}

#[gpui::test]
fn alphabet_jump_centers_selection_and_clamps_at_list_end(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "artists",
        (0..120)
            .map(|index| BrowseItem::Artist {
                key: index.to_string(),
                title: format!("Artist {index:03}"),
                thumb: None,
                is_placeholder: false,
            })
            .collect(),
    )];
    initial.columns = browser::columns(&state);
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..4 {
        draw(cx);
    }
    for selected in [20, 60, 119] {
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.columns[0].selected = selected;
        next.navigation_serial += 1;
        next.navigation_origin = browser::Origin::AlphabetJump;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..4 {
            draw(cx);
        }
        let row = cx
            .debug_bounds(match selected {
                20 => "browser-row-Artist 020",
                60 => "browser-row-Artist 060",
                _ => "browser-row-Artist 119",
            })
            .unwrap();
        let column = cx.debug_bounds("browser-column-0").unwrap();
        if selected != 119 {
            assert!(
                (row.center().y - column.center().y).abs() < px(40.),
                "{row:?} {column:?}"
            );
        } else {
            assert!(
                row.bottom() <= column.bottom() + px(1.),
                "{row:?} {column:?}"
            );
            assert!(row.center().y > column.center().y);
        }
    }
}

#[gpui::test]
fn single_window_transport_keeps_alignment_between_sidebars(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
    });
    let mut initial = snapshot();
    initial.now_playing = "A current track".into();
    initial.artist = "Artist name".into();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..5 {
        draw(cx);
    }
    assert!(cx.debug_bounds("queue-rail-false").is_some());
    assert!(cx.debug_bounds("play-queue").is_none());
    assert!(cx.debug_bounds("queue-tools").is_some());
    let rail = cx.debug_bounds("library-queue-rail").unwrap();
    for history in [false, true] {
        let shortcut = cx
            .debug_bounds(if history {
                "queue-rail-true"
            } else {
                "queue-rail-false"
            })
            .unwrap();
        cx.simulate_click(shortcut.center(), Default::default());
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command|
            matches!(command, Command::GuardedActions {actions, ..}
                if matches!(actions.as_slice(), [crate::app::Action::Queue(crate::app::action::QueueAction::ShowHistory(tab)), crate::app::Action::Navigation(crate::app::action::NavigationAction::ToggleQueuePanel)] if *tab == history))));
    }
    let history = cx.debug_bounds("queue-rail-true").unwrap();
    let artwork = cx.debug_bounds("transport-cover-slot").unwrap();
    let seek = cx.debug_bounds("transport-seek-row").unwrap();
    assert!(
        artwork.bottom() <= seek.center().y,
        "artwork overlaps the seek track: {artwork:?} {seek:?}"
    );
    let info = cx.debug_bounds("transport-info").unwrap();
    let content = cx.debug_bounds("transport-track-text").unwrap();
    assert!(
        (content.center().x - info.center().x).abs() < px(2.),
        "Track information must be centered: {content:?} {info:?}"
    );
    assert!(
        content.bottom() + px(6.) <= seek.center().y,
        "Track information needs space above seek track: {content:?} {seek:?}"
    );
    let library = cx.debug_bounds("transport-play").unwrap();
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.player_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    let transport = cx.debug_bounds("transport-bar").unwrap();
    let mut intermediate = None;
    for _ in 0..60 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
        assert_eq!(cx.debug_bounds("queue-rail-true"), Some(history));
        if let Some(sheet) = cx.debug_bounds("now-playing-sheet")
            && sheet.top() < transport.top() - px(4.)
            && sheet.bottom() > transport.top() + px(4.)
        {
            intermediate = Some(sheet);
        }
    }
    let moving = intermediate.expect("sheet passes behind the stationary transport");
    let settled = cx.debug_bounds("now-playing-sheet").unwrap();
    assert!((moving.size.height - settled.size.height).abs() < px(2.));
    assert_eq!(cx.debug_bounds("transport-bar").unwrap(), transport);
    let playing = cx.debug_bounds("transport-play").unwrap();
    assert!(
        (playing.center().x - library.center().x).abs() < px(3.),
        "{library:?} {playing:?}"
    );

    assert!(cx.debug_bounds("collapse-player-sheet").is_some());
    assert!(cx.debug_bounds("play-queue").is_none());
    let sheet = cx.debug_bounds("now-playing-sheet").unwrap();
    let sidebar = cx.debug_bounds("navigation-sidebar").unwrap();
    assert!(
        sheet.left() >= sidebar.right() - px(2.) && (sheet.right() - rail.left()).abs() <= px(2.),
        "sheet={sheet:?} sidebar={sidebar:?} rail={rail:?}"
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.queue_open = true;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..20 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
    }
    let queue = cx.debug_bounds("play-queue").unwrap();
    let tools = cx
        .debug_bounds("queue-tools")
        .expect("Now Playing queue tools menu");
    let mode = cx.debug_bounds("queue-mode").expect("queue mode menu");
    assert!(tools.left() >= queue.right());
    assert!(mode.bottom() <= tools.top());
    let bar = cx.debug_bounds("transport-bar").unwrap();
    assert!(queue.bottom() <= bar.top() && tools.bottom() <= bar.top());
    let boundary = cx.debug_bounds("workspace-content").unwrap().right();
    let from = gpui::point(boundary - px(2.), px(100.));
    let to = gpui::point(boundary - px(120.), px(100.));
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(boundary + px(10.), px(100.)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        to,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
    for _ in 0..5 {
        draw(cx);
    }
    let resized = cx.debug_bounds("transport-play").unwrap();
    assert!((resized.center().x - playing.center().x).abs() < px(3.));
    for active in [false, true, false, true] {
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.snapshot.player.active = active;
                cx.notify();
            })
        });
        for frame in 0..20 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            draw(cx);
            let bounds = cx.debug_bounds("transport-play").unwrap();
            assert!(
                (bounds.center().x - resized.center().x).abs() < px(1.),
                "active={active} frame={frame}: {resized:?} -> {bounds:?}"
            );
        }
    }
}

#[gpui::test]
fn now_playing_metadata_fits_and_transport_uses_the_available_width(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.queue_open = true;
    initial.now_playing = "Outroduction to Diagnosis Psychosis".into();
    initial.status = "Remix: Twofer processing…".into();
    initial.player.track = Some(
        serde_json::from_value(serde_json::json!({
            "title": "Outroduction to Diagnosis Psychosis", "originalTitle": "Prince Paul",
            "parentTitle": "Psychoanalysis: What Is It?", "year": 1996,
            "genres": ["Hip Hop"], "duration": 279000, "index": 17,
            "Media": [{"bitrate":320,"audioChannels":2,"audioCodec":"mp3","container":"mp3"}]
        }))
        .unwrap(),
    );
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1300.), px(820.)));
    for _ in 0..20 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
    }
    let info = cx.debug_bounds("now-playing-metadata").unwrap();
    for label in [
        "now-playing-field-Year",
        "now-playing-field-Duration",
        "now-playing-field-Track",
        "now-playing-field-Bit rate",
        "now-playing-field-Channels",
        "now-playing-field-Format",
    ] {
        let field = cx.debug_bounds(label).expect(label);
        assert!(
            field.bottom() <= info.bottom(),
            "{label}: {field:?} outside {info:?}"
        );
    }
    let bar = cx.debug_bounds("transport-bar").unwrap();
    let seek = cx.debug_bounds("transport-seek-row").unwrap();
    let info = cx.debug_bounds("transport-info").unwrap();
    assert!(
        seek.size.width >= info.size.width * 0.9
            && seek.left() >= info.left()
            && seek.right() <= info.right(),
        "{seek:?} outside {info:?}, bar {bar:?}"
    );
    let play = cx.debug_bounds("transport-play").unwrap();
    let volume = cx.debug_bounds("transport-volume").unwrap();
    assert!(play.right() < seek.left(), "{play:?} {bar:?}");
    assert!(volume.left() > play.right());
    assert!((volume.center().y - play.center().y).abs() < px(2.));
    let sidebar = cx.debug_bounds("toggle-sidebar").unwrap();
    let reveal = cx.debug_bounds("queue-rail-false").unwrap();
    assert!(sidebar.bottom() < bar.top() && reveal.bottom() < bar.top());
    assert_eq!(bar.left(), px(1300.) - bar.right());
    assert!(sidebar.right() < info.left() && reveal.left() > info.right());
    assert_eq!(sidebar.bottom(), reveal.bottom());
    assert!(reveal.size.width >= px(30.));
    assert!(reveal.right() < px(1300.));
    assert!(cx.debug_bounds("transport-queue").is_none());
    assert!(cx.debug_bounds("utility-item-count").is_none());
    for width in [1050., 880., 1300.] {
        cx.simulate_resize(gpui::size(px(width), px(820.)));
        draw(cx);
        draw(cx);
        let info = cx.debug_bounds("transport-info").unwrap();
        let text = cx.debug_bounds("transport-track-text").unwrap();
        let seek = cx.debug_bounds("transport-seek-row").unwrap();
        assert!(info.size.width >= px(220.), "width={width}: {info:?}");
        assert!(text.size.width >= px(150.), "width={width}: {text:?}");
        assert!(seek.size.width >= px(200.), "width={width}: {seek:?}");
        assert!(text.left() >= info.left() && text.right() <= info.right());
    }
    while commands.try_recv().is_ok() {}
    let metadata = cx.debug_bounds("transport-track-text").unwrap();
    cx.simulate_click(metadata.center(), Default::default());
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(c, Command::Palette(_)))
    );
    let title = cx.debug_bounds("now-playing-track-title").unwrap();
    cx.simulate_click(title.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Palette(crate::app::state::PaletteCommandKind::FromTrackContext {
                kind: crate::app::track_context::ContextKind::ShowDetails(
                    crate::app::library_details::Destination::Track
                ),
                ..
            })
        ))
    );
}

#[gpui::test]
fn fullscreen_modes_cycle_and_only_transport_modes_show_up_next(cx: &mut TestAppContext) {
    use crate::app::state::DesktopFullscreenView as Mode;
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.player.fullscreen = true;
    initial.player.viewer = Mode::Artwork;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        captured.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(px(1300.), px(820.)));
    for mode in Mode::MODES {
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.snapshot.player.viewer = mode;
                view.snapshot.player.mode = if mode.is_artwork() { 0 } else { 1 };
                cx.notify();
            })
        });
        for _ in 0..5 {
            draw(cx);
        }
        assert!(cx.debug_bounds("fullscreen-content").is_some());
        assert_eq!(
            cx.debug_bounds("fullscreen-transport").is_some(),
            mode.has_transport()
        );
        assert_eq!(
            cx.debug_bounds("up-next-empty").is_some(),
            mode.has_transport()
        );
        while commands.try_recv().is_ok() {}
        cx.simulate_keystrokes("right");
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Palette(crate::app::state::PaletteCommandKind::DesktopFullscreen(next)) if next == mode.cycle(true)
        )));
        assert_eq!(mode.cycle(true).cycle(false), mode);
    }
}

#[gpui::test]
fn up_next_fits_collapsed_queue_and_opens_without_playing(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    state.queue.tracks = [
        "Current occurrence",
        "Next occurrence with a long title 日本語",
    ]
    .map(|title| crate::library::models::Track {
        rating_key: "same-recording".into(),
        title: title.into(),
        original_title: Some("An artist with a long name".into()),
        ..Default::default()
    })
    .into();
    state.queue.index = Some(0);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.player.fullscreen = true;
    initial.player.viewer = crate::app::state::DesktopFullscreenView::ArtworkTransport;
    initial.playback_queue = Some(browser::queue(&state));
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..5 {
        draw(cx);
    }
    let panel = cx
        .debug_bounds("up-next-panel")
        .expect("collapsed queue preview");
    let row = cx.debug_bounds("up-next-track").expect("next track");
    let title = cx.debug_bounds("up-next-title").unwrap();
    let artist = cx.debug_bounds("up-next-artist").unwrap();
    assert!(row.left() >= panel.left() && row.right() <= panel.right());
    assert!(row.top() >= panel.top() && row.bottom() <= panel.bottom());
    assert!(title.size.width > px(0.) && title.right() <= panel.right());
    assert!(artist.size.width > px(0.) && artist.right() <= panel.right());
    cx.simulate_click(row.center(), Default::default());
    let mut opened = false;
    while let Ok(command) = commands.try_recv() {
        assert!(!matches!(
            command,
            Command::Select {
                intent: browser::Intent::Play | browser::Intent::PlayFollowing,
                ..
            }
        ));
        opened |= matches!(
            command,
            Command::Palette(crate::app::state::PaletteCommandKind::GotoNowPlaying)
        );
    }
    assert!(opened, "preview opens the existing Now Playing queue");
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.snapshot.playback_queue.as_mut().unwrap().playing = Some(1);
            cx.notify();
        })
    });
    for _ in 0..3 {
        draw(cx);
    }
    assert!(
        cx.debug_bounds("up-next-empty").is_some(),
        "last occurrence has no following track"
    );
    assert!(cx.debug_bounds("up-next-track").is_none());
}

#[gpui::test]
fn up_next_shows_first_unstarted_track_and_empty_queue(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.player.fullscreen = true;
    initial.player.viewer = crate::app::state::DesktopFullscreenView::ArtworkTransport;
    let mut state = AppState::new();
    state.queue.tracks.push(crate::library::models::Track {
        title: "First queued track".into(),
        ..Default::default()
    });
    initial.playback_queue = Some(browser::queue(&state));
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..5 {
        draw(cx);
    }
    assert!(cx.debug_bounds("up-next-track").is_some());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.snapshot.playback_queue = None;
            cx.notify();
        })
    });
    for _ in 0..3 {
        draw(cx);
    }
    assert!(cx.debug_bounds("up-next-empty").is_some());
}

#[gpui::test]
fn column_heading_compacts_when_constrained_and_restores_when_short(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.columns[0].named_heading = true;
    initial.columns[0].title =
        "tracks — A very long album name 日本語 with an important distinguishing suffix 78/79"
            .into();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(820.), px(600.)));
    for _ in 0..8 {
        draw(cx);
    }
    assert!(cx.debug_bounds("column-heading-0-small").is_some());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.snapshot.columns[0].title = "tracks — Short".into();
            cx.notify();
        })
    });
    for _ in 0..5 {
        draw(cx);
    }
    assert!(cx.debug_bounds("column-heading-0-full").is_some());
}

#[gpui::test]
fn radio_child_in_player_sidebar_and_transport_disclosure(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    for (key, title) in [
        (None, "Stations"),
        (Some("decades".to_string()), "Decade Mix"),
    ] {
        state
            .station_nav
            .push_column(crate::app::state::StationColumn::new(
                key,
                title.into(),
                vec![],
            ));
    }
    let mut initial = snapshot();
    initial.radio_browser = Some(super::surface::radio_surface(&state));
    initial.queue_open = true;
    let (engine, updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    cx.update(|w, cx| view.update(cx, |v, cx| v.apply_snapshot(initial.clone(), w, cx)));
    for _ in 0..40 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
    }
    let up = cx.debug_bounds("expand-player-sheet").unwrap();
    cx.simulate_click(up.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
            c,
            Command::Palette(crate::app::state::PaletteCommandKind::GotoNowPlaying)
        ))
    );
    initial.sheet_browser = true;
    initial.player_window_request += 1;
    updates.send_replace(initial.clone());
    cx.update(|w, cx| {
        view.update(cx, |v, cx| {
            v.apply_snapshot(initial, w, cx);
            v.sheet_sidebar_root = false;
        })
    });
    for _ in 0..40 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
    }
    let down = cx.debug_bounds("collapse-player-sheet").unwrap();
    assert!(
        (down.center().x - up.center().x).abs() <= px(2.),
        "{up:?} {down:?}"
    );
    let child = cx.debug_bounds("radio-column-1").unwrap();
    assert!(child.size.width >= cx.debug_bounds("navigation-sidebar").unwrap().size.width - px(4.));
    assert!(cx.debug_bounds("radio-column-0").is_none());
    assert!(cx.debug_bounds("expand-player-sheet").is_none());
    let back = cx.debug_bounds("sheet-browser-back").unwrap();
    cx.simulate_click(back.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
            c,
            Command::Action(crate::app::Action::Radio(
                crate::app::action::RadioAction::NavigateStationsBack
            ))
        ))
    );
}

#[gpui::test]
fn radio_columns_are_library_content_in_both_window_modes(cx: &mut TestAppContext) {
    cx.update(init);
    for separate in [false, true] {
        let mut state = AppState::new();
        state.view = crate::app::View::Browse;
        state.desktop_library_root = crate::app::state::LibraryRoot::Radio;
        state
            .station_nav
            .columns
            .push(crate::app::state::StationColumn::new(
                None,
                "Stations".into(),
                crate::library::station::standard_stations(
                    "fixture",
                    crate::library::capabilities::NAVIDROME,
                ),
            ));
        let mut initial = snapshot();
        initial.separate_player = separate;
        initial.radio_browser = Some(super::surface::radio_surface(&state));
        initial.sidebar = super::catalog::sidebar(&state);
        let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
        let engine = std::sync::Arc::new(engine);
        let (_view, cx) = cx.add_window_view(move |window, cx| {
            let mut view = Desktop::new(
                engine,
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        draw(cx);
        draw(cx);
        let library = cx.debug_bounds("library-workspace").unwrap();
        let radio = cx
            .debug_bounds("radio-browser")
            .expect("Radio occupies library workspace");
        assert!(radio.size.height > library.size.height - px(5.));
        assert!(cx.debug_bounds("radio-column-0").is_some());
        let row = cx.debug_bounds("radio-row-0-0").unwrap();
        cx.simulate_mouse_down(row.center(), gpui::MouseButton::Left, Default::default());
        cx.simulate_mouse_up(row.center(), gpui::MouseButton::Left, Default::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
                command,
                Command::Surface {
                    operation: super::surface::Operation::RadioSelect { column: 0, .. },
                    ..
                }
            ))
        );
    }
}

#[gpui::test]
fn leaving_inline_forms_restores_keyboard_focus(cx: &mut TestAppContext) {
    cx.update(init);
    for player in [false, true] {
        let mut state = AppState::new();
        state.view = crate::app::View::Browse;
        state.desktop_library_root = crate::app::state::LibraryRoot::DescribeMusic;
        state
            .popups
            .set_input_dialog(Some(crate::app::state::InputDialog {
                title: "Describe music…".into(),
                input: "".into(),
                action_type: crate::app::state::InputDialogAction::AudioMuseSearch(
                    crate::audiomuse::Feature::Describe,
                ),
            }));
        let mut initial = snapshot();
        initial.workspace = super::workspace::project(&state);
        initial.workspace_title = Some("Describe Music".into());
        let (engine, _updates, _commands) = Engine::fixture(initial.clone());
        let before = initial.clone();
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = before;
            view
        });
        draw(cx);
        draw(cx);
        assert!(
            !cx.update(|window, cx| view.read(cx).focus.is_focused(window)),
            "the inline input should initially own focus"
        );
        let mut next = initial;
        if player {
            next.player_window_request = 1;
        } else {
            next.workspace = None;
            next.workspace_title = None;
        }
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        draw(cx);
        assert!(
            cx.update(|window, cx| view.read(cx).focus.is_focused(window)),
            "removed/hidden inputs must return focus to the desktop"
        );
    }
}

#[gpui::test]
fn description_prompt_stays_with_history_and_dispatches_collection_actions(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let mut state = AppState::new();
    state.desktop_library_root = crate::app::state::LibraryRoot::DescribeMusic;
    state.sources.nav_collection = Some(
        crate::app::sources::navidrome::commands::CollectionKind::AudioMuse(
            crate::audiomuse::Feature::Describe,
        ),
    );
    let tracks: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|id| crate::library::track::Track {
            rating_key: id.into(),
            title: id.into(),
            ..Default::default()
        })
        .collect();
    state.artist_nav.columns = vec![
        BrowseColumn::new(
            "Description history",
            vec![BrowseItem::Description {
                query: "Quiet piano".into(),
            }],
        ),
        BrowseColumn::new_with_tracks("Quiet piano", BrowseItem::from_tracks(&tracks), tracks),
    ];
    let mut initial = snapshot();
    initial.columns = browser::columns(&state);
    initial.workspace_title = Some("Describe Music".into());
    initial.generation = 17;
    assert_eq!(initial.columns[1].header_controls.len(), 2);
    assert!(matches!(&initial.columns[1].header_controls[0].actions[0],
        crate::app::Action::Queue(crate::app::action::QueueAction::PlayTracksNow(tracks))
        if tracks.iter().map(|t| t.rating_key.as_str()).collect::<Vec<_>>() == ["first", "second"]));
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    draw(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.description_input.update(cx, |input, cx| {
                input.set_value("Warm acoustic music", window, cx)
            });
        });
    });
    draw(cx);
    draw(cx);
    while commands.try_recv().is_ok() {}
    let button = cx.debug_bounds("submit-description").unwrap();
    cx.simulate_click(button.center(), Default::default());
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
        Command::GuardedActions { generation:17, actions, .. }
        if matches!(actions.as_slice(), [crate::app::Action::Source(crate::app::sources::SourceAction::AudioMuse(action))]
            if matches!(action.as_ref(), crate::app::sources::audiomuse::Command::Search { query, .. } if query == "Warm acoustic music")))));
    assert!(cx.debug_bounds("description-history").is_none());
}

#[gpui::test]
fn visualizer_stays_visible_at_its_initial_minimum_and_tabs_fit_their_labels(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    initial.player.active = true;
    initial.queue_open = true;
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1600.), px(900.)));
    for _ in 0..5 {
        draw(cx);
    }
    let row = cx.debug_bounds("visualizer-mode-row").unwrap();
    let tabs = cx.debug_bounds("visualizer-mode-tabs").unwrap();
    let expand = cx.debug_bounds("expand-visualizer").unwrap();
    assert!(tabs.left() >= row.left() && tabs.right() <= expand.left());
    assert!(expand.right() <= row.right());
    let queue = cx.debug_bounds("play-queue").unwrap();
    let toggle = cx.debug_bounds("toggle-visualizer").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    draw(cx);
    assert!(cx.debug_bounds("visualizer-pane").is_none());
    cx.simulate_click(toggle.center(), Default::default());
    draw(cx);
    let initial = cx.debug_bounds("visualizer-pane").unwrap();
    let handle = gpui::point(initial.center().x, initial.top());
    let enlarged = handle - gpui::point(px(0.), px(100.));
    for destination in [enlarged, handle + gpui::point(px(0.), px(200.))] {
        let pane = cx.debug_bounds("visualizer-pane").unwrap();
        let from = gpui::point(pane.center().x, pane.top());
        cx.simulate_mouse_move(from, None, Default::default());
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
        cx.simulate_mouse_move(
            from - gpui::point(px(0.), px(10.)),
            gpui::MouseButton::Left,
            Default::default(),
        );
        cx.simulate_mouse_move(destination, gpui::MouseButton::Left, Default::default());
        cx.simulate_mouse_up(destination, gpui::MouseButton::Left, Default::default());
        for _ in 0..5 {
            draw(cx);
        }
        let height = cx.debug_bounds("visualizer-pane").unwrap().size.height;
        if destination == enlarged {
            assert!(
                height > initial.size.height + px(50.),
                "{height:?} {initial:?}"
            );
        } else {
            assert!(
                (height - initial.size.height).abs() < px(2.),
                "{height:?} {initial:?}"
            );
        }
        assert!(cx.debug_bounds("visualizer-content").is_some());
        assert!(cx.debug_bounds("visualizer-mode-tabs").is_some());
        assert_eq!(cx.debug_bounds("play-queue").unwrap(), queue);
    }
}

#[gpui::test]
fn search_reopens_after_toolbar_dismissal_even_when_closed_snapshot_is_coalesced(
    cx: &mut TestAppContext,
) {
    use gpui_kit::component::WindowExt;
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    for playing in [false, true] {
        let mut state = AppState::new();
        state.popups.set_search_active(true);
        let model = super::surface::project(&state).unwrap();
        let mut initial = snapshot();
        initial.player.active = playing;
        let (engine, _updates, _commands) = Engine::fixture(initial.clone());
        let engine = std::sync::Arc::new(engine);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let captured = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                let mut desktop = Desktop::new(
                    engine,
                    WindowRole::Library,
                    WindowRegistry::default(),
                    window,
                    cx,
                );
                desktop.snapshot = initial;
                desktop
            });
            captured.replace(Some(view.clone()));
            gpui_kit::base::Root::new(view, window, cx)
        });
        let view = slot.borrow().clone().unwrap();
        for request in 1..=4 {
            let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
            next.surface = Some(model.clone());
            next.surface_request = request;
            cx.update(|window, cx| {
                view.update(cx, |view, cx| view.apply_snapshot(next, window, cx))
            });
            for _ in 0..3 {
                draw(cx);
            }
            cx.update(|window, cx| {
                assert!(
                    window.has_active_dialog(cx),
                    "playing={playing} request={request}"
                )
            });
            let button = cx.debug_bounds("sidebar-search").unwrap();
            cx.simulate_click(button.center(), Default::default());
            draw(cx);
            cx.run_until_parked();
            cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        }
    }
}

#[gpui::test]
fn now_playing_keeps_the_focused_column_and_steps_back(cx: &mut TestAppContext) {
    cx.update(init);
    let mut library = snapshot();
    let mut tracks = library.columns[0].clone();
    tracks.identity = "album-tracks".into();
    library.columns.push(tracks);
    library.focused_column = 1;
    let (engine, _updates, mut commands) = Engine::fixture(library.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = library;
        view
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    for _ in 0..4 {
        draw(cx);
    }
    let sidebar = cx.debug_bounds("navigation-sidebar").unwrap();
    let mut player = view.read_with(cx, |v, _| v.snapshot.clone());
    player.sheet_browser = true;
    player.player_window_request += 1;
    cx.update(|w, cx| view.update(cx, |v, cx| v.apply_snapshot(player, w, cx)));
    for _ in 0..40 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
        assert_eq!(cx.debug_bounds("navigation-sidebar").unwrap(), sidebar);
        if let Some(sheet) = cx.debug_bounds("now-playing-sheet") {
            assert_eq!(sheet.left(), sidebar.right());
        }
    }
    assert!(cx.debug_bounds("browser-column-1").is_some());
    assert!(cx.debug_bounds("browser-column-0").is_none());
    for (preferred, key, configures) in [
        (None, "randomAlbum", false),
        (Some("artist-picker"), "artist-picker", true),
    ] {
        view.update(cx, |view, cx| {
            view.snapshot.library_stations = crate::library::station::standard_stations(
                "fixture",
                crate::library::capabilities::NAVIDROME,
            );
            view.snapshot.preferred_station = preferred.map(str::to_owned);
            cx.notify();
        });
        draw(cx);
        let right = cx.debug_bounds("library-queue-rail").unwrap();
        for icon in [
            "sidebar-radio",
            "sidebar-search",
            "sidebar-settings",
            "sidebar-palette",
            "queue-rail-true",
        ] {
            let bounds = cx.debug_bounds(icon).unwrap();
            assert!(
                bounds.left() >= right.left() && bounds.right() <= right.right(),
                "{icon} outside the right rail"
            );
        }
        let radio = cx.debug_bounds("sidebar-radio").unwrap();
        cx.simulate_click(radio.center(), Default::default());
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|cmd| match cmd {
                Command::Palette(crate::app::state::PaletteCommandKind::ConfigureStation(
                    station,
                )) => configures && station == format!("fixture/{key}"),
                Command::Palette(crate::app::state::PaletteCommandKind::PlayStation(station)) =>
                    !configures && station == format!("fixture/{key}"),
                _ => false,
            })
        );
    }
    let back = cx.debug_bounds("sheet-browser-back").unwrap();
    let toggle = cx.debug_bounds("toggle-sidebar").unwrap();
    assert_eq!(toggle.center().y, back.center().y);
    assert!(back.left() > toggle.right() && sidebar.right() - back.right() <= px(12.));
    cx.simulate_click(back.center(), Default::default());
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(
        |cmd| matches!(cmd,Command::CloseColumn {column:1,identity,..} if identity=="album-tracks")
    ));
    let mut parent = view.read_with(cx, |v, _| v.snapshot.clone());
    parent.columns.pop();
    parent.focused_column = 0;
    cx.update(|w, cx| view.update(cx, |v, cx| v.apply_snapshot(parent, w, cx)));
    draw(cx);
    cx.simulate_click(back.center(), Default::default());
    draw(cx);
    assert!(cx.debug_bounds("sidebar-Artists").is_some());
    assert!(cx.debug_bounds("collapse-player-sheet").is_some());
}

#[gpui::test]
fn queue_stays_open_across_player_and_library_navigation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
        cx.bind_keys([gpui::KeyBinding::new(
            "down",
            super::MoveDown,
            Some("Textamp"),
        )]);
    });
    let mut initial = snapshot();
    initial.columns.push(initial.columns[0].clone());
    initial.focused_column = 1;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    for _ in 0..5 {
        draw(cx);
    }
    assert!(cx.debug_bounds("browser-column-0").is_some());
    assert!(cx.debug_bounds("play-queue").is_none());
    let sidebar = cx.debug_bounds("navigation-sidebar").unwrap();
    let library_artists = cx.debug_bounds("sidebar-Artists").unwrap();
    let button = cx.debug_bounds("queue-rail-false").unwrap();
    while commands.try_recv().is_ok() {}
    cx.simulate_click(button.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::GuardedActions{actions,..} if matches!(actions.last(), Some(crate::app::Action::Navigation(crate::app::action::NavigationAction::ToggleQueuePanel)))))
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.queue_open = true;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    let queue = cx.debug_bounds("queue-dock").unwrap();
    assert!((queue.size.width - sidebar.size.width).abs() <= px(2.));
    assert!(cx.debug_bounds("now-playing-sheet").is_none());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.player_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert!(cx.debug_bounds("play-queue").is_some());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    let original_player = cx.debug_bounds("sheet-player-content").unwrap();
    let original_queue = cx.debug_bounds("play-queue").unwrap();
    cx.update(|_, cx| cx.set_reduce_motion(false));
    next.sheet_browser = true;
    next.library_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    let mut moved = false;
    for _ in 0..60 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        draw(cx);
        if let Some(browser) = cx.debug_bounds("navigation-sidebar")
            && browser.size.width > px(super::motion::SIDEBAR_RAIL_WIDTH + 1.)
            && browser.size.width < sidebar.size.width - px(1.)
        {
            let player = cx.debug_bounds("sheet-player-content").unwrap();
            assert_eq!(browser.right(), player.left());
            assert_eq!(cx.debug_bounds("play-queue").unwrap(), original_queue);
            assert_eq!(browser.left(), sidebar.left());
            assert!(cx.debug_bounds("sidebar-Artists").is_some());
            moved = true;
        }
    }
    assert!(
        moved,
        "the browser must travel between the rail and its expanded position"
    );
    cx.update(|_, cx| cx.set_reduce_motion(true));
    assert!(cx.debug_bounds("browser-column-0").is_none());
    assert_eq!(cx.debug_bounds("sidebar-Artists").unwrap(), library_artists);
    cx.simulate_click(library_artists.center(), Default::default());
    draw(cx);
    assert!(cx.debug_bounds("browser-column-1").is_some());
    assert!(cx.debug_bounds("play-queue").is_some());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    let browser = cx.debug_bounds("navigation-sidebar").unwrap();
    let player = cx.debug_bounds("sheet-player-content").unwrap();
    assert_eq!(browser.size.width, sidebar.size.width);
    assert_eq!(browser.left(), sidebar.left());
    assert_eq!(browser.right(), player.left());
    assert_eq!(player.right(), original_player.right());
    assert_eq!(original_queue, cx.debug_bounds("play-queue").unwrap());
    assert!(cx.debug_bounds("transport-artwork").unwrap().left() >= player.left());
    next.sheet_browser = false;
    next.library_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert_eq!(cx.debug_bounds("play-queue"), Some(original_queue));
    assert!(cx.debug_bounds("library-back").is_none());
    assert!(cx.debug_bounds("browser-column-0").is_some());
    assert!(cx.debug_bounds("browser-column-1").is_some());
    assert!(
        view.read_with(cx, |view, _| view.snapshot.columns[0].selected == 0
            && !view.snapshot.player.active)
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.navigation_origin = browser::Origin::LibraryJump;
    next.navigation_serial += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    assert!(!view.read_with(cx, |view, _| view.queue_focused));
}

#[gpui::test]
fn mute_projection_moves_slider_to_zero_and_restores_it(cx: &mut TestAppContext) {
    cx.update(init);
    let initial = snapshot();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for (muted, volume) in [(false, 0.62), (true, 0.), (false, 0.62)] {
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.muted = muted;
        next.volume = volume;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..3 {
            draw(cx);
        }
        view.read_with(cx, |view, cx| {
            assert!((view.volume_slider.read(cx).value().end() - volume).abs() < 0.0001)
        });
    }
}

#[gpui::test]
fn failed_empty_library_retry_dispatches_refresh(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.columns.clear();
    initial.error = Some("Connection unavailable".into());
    initial.libraries = vec![crate::app::sources::LibraryChoice::Folder(
        crate::library::FolderSource {
            id: "fixture".into(),
            name: "Fixture".into(),
            location: crate::library::FolderLocation::Local {
                path: "/fixture".into(),
            },
        },
    )];
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    let retry = cx.debug_bounds("retry-library").expect("retry control");
    cx.simulate_click(retry.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Palette(crate::app::state::PaletteCommandKind::Refresh)
        ))
    );
    assert!(cx.debug_bounds("library-empty-wordmark").is_none());
    view.update(cx, |v, cx| {
        v.snapshot.error = None;
        cx.notify();
    });
    draw(cx);
    let logo = cx.debug_bounds("library-empty-wordmark").unwrap();
    assert!(logo.size.width > px(0.) && logo.size.height > px(0.));
    assert!(cx.debug_bounds("retry-library").is_none());
}

#[gpui::test]
fn loading_playlist_uses_utility_progress_without_an_empty_state(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.columns.clear();
    initial.section = "Playlists".into();
    initial.loading = Some("Loading playlist…".into());
    initial.status = "Earlier status".into();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..5 {
        draw(cx);
    }
    assert!(cx.debug_bounds("library-empty-state").is_none());
    assert!(cx.debug_bounds("utility-loading").is_some());
}

#[gpui::test]
fn clearing_column_filter_reveals_the_selected_original_row(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "artists",
        (0..120)
            .map(|index| BrowseItem::Artist {
                key: format!("artist-{index}"),
                title: format!("Artist {index:03}"),
                thumb: None,
                is_placeholder: false,
            })
            .collect(),
    )];
    let mut initial = snapshot();
    initial.columns = browser::columns(&state);
    initial.filter.active = true;
    initial.filter.query = "artist".into();
    initial.filter.results = Some(crate::app::state::ListFilterResults {
        matched_indices: vec![10, 80, 119],
        total_matches: 3,
        has_more: false,
    });
    initial.columns[0].selected = 10;
    let filtered = initial.clone();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for (selected, active) in [(80, false), (119, true)] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.apply_snapshot(filtered.clone(), window, cx)
            })
        });
        for _ in 0..5 {
            draw(cx);
        }
        while commands.try_recv().is_ok() {}
        let label = if selected == 80 {
            "browser-row-Artist 080"
        } else {
            "browser-row-Artist 119"
        };
        let row = cx.debug_bounds(label).unwrap();
        cx.simulate_click(row.center(), Default::default());
        draw(cx);
        assert_eq!(
            cx.debug_bounds(label).unwrap(),
            row,
            "Clicking must not recenter the filtered list"
        );
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Select { target, .. } if target.row == selected))
        );
        let mut next = filtered.clone();
        next.filter = Default::default();
        next.filter.active = active;
        next.columns[0].selected = selected;
        next.navigation_origin = browser::Origin::Pointer;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..5 {
            draw(cx);
        }
        let row = cx
            .debug_bounds(label)
            .expect("Selected row remains in the viewport after clearing");
        let column = cx.debug_bounds("browser-column-0").unwrap();
        assert!(row.top() >= column.top() && row.bottom() <= column.bottom());
        if selected == 80 {
            assert!((row.center().y - column.center().y).abs() < px(40.));
        }
        view.read_with(cx, |view, _| {
            assert_eq!(view.snapshot.columns[0].selected, selected)
        });
    }
}

#[gpui::test]
fn library_jump_centers_every_selected_ancestor_and_clamps_the_last_row(cx: &mut TestAppContext) {
    use gpui_kit::component::scroll::ScrollbarHandle;
    cx.update(init);
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = ["Parent", "Child"]
        .into_iter()
        .map(|prefix| {
            BrowseColumn::new(
                prefix,
                (0..120)
                    .map(|index| BrowseItem::Artist {
                        key: format!("{prefix}-{index}"),
                        title: format!("{prefix} {index:03}"),
                        thumb: None,
                        is_placeholder: false,
                    })
                    .collect(),
            )
        })
        .collect();
    let mut initial = snapshot();
    initial.columns = browser::columns(&state);
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..5 {
        draw(cx);
    }
    for selection in [[40, 70], [119, 119]] {
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.navigation_serial += 1;
        next.navigation_origin = browser::Origin::LibraryJump;
        for (column, selected) in next.columns.iter_mut().zip(selection) {
            column.selected = selected;
        }
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..5 {
            draw(cx);
        }
        for (index, selected) in selection.into_iter().enumerate() {
            let row = cx
                .debug_bounds(match (index, selected) {
                    (0, 40) => "browser-row-Parent 040",
                    (1, 70) => "browser-row-Child 070",
                    (0, _) => "browser-row-Parent 119",
                    _ => "browser-row-Child 119",
                })
                .unwrap();
            let column = cx
                .debug_bounds(if index == 0 {
                    "browser-column-0"
                } else {
                    "browser-column-1"
                })
                .unwrap();
            if selected == 119 {
                assert!(row.bottom() <= column.bottom() + px(1.));
            } else {
                assert!(
                    (row.center().y - column.center().y).abs() < px(40.),
                    "{row:?} {column:?}"
                );
            }
        }
    }
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.navigation_serial += 1;
    next.navigation_origin = browser::Origin::LibraryJump;
    next.focused_column = 1;
    next.columns[1].identity = "album-tracklist".into();
    next.columns[1].selected = 0;
    next.columns[1].album_info = true;
    next.columns[1].album_info_selected = true;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..5 {
        draw(cx);
    }
    view.read_with(cx, |view, _| {
        let column = &view.snapshot.columns[1];
        assert_eq!(
            view.scroll_handles[&column.identity].offset().y,
            px(0.),
            "Album jumps start at the first row without a hidden filter"
        );
        assert!(view.pending_centered_rows.is_empty());
    });
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let scroll = &view.scroll_handles["album-tracklist"];
            scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        });
    });
    for _ in 0..3 {
        draw(cx);
    }
    view.read_with(cx, |view, _| {
        assert_eq!(view.scroll_handles["album-tracklist"].offset().y, px(0.));
    });
}

#[gpui::test]
fn playlist_edit_button_and_checkboxes(cx: &mut TestAppContext) {
    use crate::app::{action::BrowseAction, playback_controls::Control};
    cx.update(init);
    let mut initial = snapshot();
    let c = &mut initial.columns[0];
    c.editable_playlist = true;
    let mut row = c.rows[0].clone();
    row.kind = browser::Kind::Album;
    row.key = "playlist-edit-album".into();
    c.rows = std::sync::Arc::new(vec![row]);
    c.header_controls = vec![Control::new(
        "Edit",
        BrowseAction::SetPlaylistEditing {
            playlist: "fixture".into(),
            editing: true,
        },
    )];
    let (engine, updates, mut commands) = Engine::fixture(initial.clone());
    let mut editing = initial.clone();
    editing.columns[0].playlist_editing = true;
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..3 {
        draw(cx);
    }
    let edit = cx.debug_bounds("column-control-0-0").unwrap();
    cx.simulate_click(edit.center(), Default::default());
    assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|c|matches!(c,Command::GuardedActions{actions,..} if matches!(actions.as_slice(),[crate::app::Action::Browse(BrowseAction::SetPlaylistEditing{editing:true,..})]))));
    updates.send_replace(editing);
    cx.run_until_parked();
    draw(cx);
    let title = cx
        .debug_bounds("browser-title-playlist-edit-album")
        .unwrap();
    cx.simulate_click(title.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
            c,
            Command::Select {
                mode: browser::Selection::Single,
                intent: browser::Intent::Inspect,
                ..
            }
        ))
    );
    let checkbox = cx.debug_bounds("playlist-checkbox-0-0").unwrap();
    cx.simulate_click(checkbox.center(), Default::default());
    assert_eq!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter(|c| matches!(
                c,
                Command::Select {
                    mode: browser::Selection::Check,
                    intent: browser::Intent::Focus,
                    ..
                }
            ))
            .count(),
        1
    );
}
#[gpui::test]
fn album_context_menu_opens_playlist_chooser_for_clicked_album(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "Albums",
        vec![BrowseItem::Album {
            key: "navidrome:fixture:album".into(),
            title: "Example Album".into(),
            artist: "Artist".into(),
            year: None,
            thumb: None,
            is_placeholder: false,
        }],
    )];
    let mut initial = snapshot();
    initial.columns = browser::columns(&state);
    initial.context_policy.server_playlists = true;
    initial.generation = 17;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        gpui_kit::base::Root::new(view, window, cx)
    });
    for _ in 0..5 {
        draw(cx);
    }
    let row = cx.debug_bounds("browser-row-Example Album").unwrap();
    cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
        Command::Select {target,intent:browser::Intent::AddAlbumToPlaylist,..} if target.key=="navidrome:fixture:album"
    )));
}

#[gpui::test]
fn description_history_context_menu_dispatches_guarded_removal(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    state.view = crate::app::View::Browse;
    state.artist_nav.columns = vec![BrowseColumn::new(
        "Description history",
        vec![BrowseItem::Description {
            query: "Quiet piano".into(),
        }],
    )];
    let mut initial = snapshot();
    initial.columns = browser::columns(&state);
    initial.generation = 17;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        gpui_kit::base::Root::new(view, window, cx)
    });
    for _ in 0..5 {
        draw(cx);
    }
    let row = cx.debug_bounds("browser-row-Quiet piano").unwrap();
    cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
        Command::GuardedActions { generation:17, target:Some(target), actions }
            if target.key == "Quiet piano" && matches!(actions.as_slice(),
                [crate::app::Action::Source(crate::app::sources::SourceAction::AudioMuse(action))]
                if matches!(action.as_ref(), crate::app::sources::audiomuse::Command::RemoveDescription { query } if query == "Quiet piano")
            )
    )));
}

#[gpui::test]
fn empty_waveform_hides_time_ruler_and_sidebar_toggle_reclaims_space(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    initial.sidebar.last_mut().unwrap().parent = Some(crate::app::sidebar::SidebarGroup::Playlists);
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..6 {
        draw(cx);
    }
    assert!(cx.debug_bounds("sidebar-Fixture Playlist").is_some());
    let width = cx.debug_bounds("browser-column-0").unwrap().size.width;
    let button = cx.debug_bounds("toggle-sidebar").unwrap();
    cx.simulate_click(button.center(), Default::default());
    for _ in 0..6 {
        draw(cx);
    }
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Action(crate::app::Action::Navigation(
                crate::app::action::NavigationAction::ToggleSidebar
            ))
        ))
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.sidebar_visible = Some(false);
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert!(cx.debug_bounds("sidebar-Fixture Playlist").is_none());
    let artists = cx
        .debug_bounds("sidebar-Artists")
        .expect("compact Artists destination");
    cx.simulate_click(artists.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::SidebarOpen { index: 0, .. }))
    );
    assert!(!view.read_with(cx, |view, _| view.sidebar_visible));
    assert!(cx.debug_bounds("browser-column-0").unwrap().size.width > width + px(100.));
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.player.active = true;
    next.player_window_request += 1;
    next.player.mode = 1;
    next.duration_ms = 60_000;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    let rail = cx.debug_bounds("navigation-sidebar").unwrap();
    let sheet = cx.debug_bounds("now-playing-sheet").unwrap();
    assert!((rail.right() - sheet.left()).abs() < px(2.));
    let artists = cx.debug_bounds("sidebar-Artists").unwrap();
    cx.simulate_click(artists.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::SidebarOpen { index: 0, .. }))
    );
    assert!(cx.debug_bounds("waveform-time-ruler").is_none());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.player.waveform = vec![0.2; 128];
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert!(cx.debug_bounds("waveform-time-ruler").is_some());
}

#[gpui::test]
fn queue_reveal_preserves_preferred_width_through_animation_and_reversal(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(false);
    });
    let mut initial = snapshot();
    initial.player.active = true;
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.presentation = WindowRole::NowPlaying;
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    for _ in 0..4 {
        draw(cx);
    }
    let width = view.read_with(cx, |view, _| view.queue_width);
    for (open, frames) in [(true, 10), (false, 4), (true, 60)] {
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.queue_open = open;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..frames {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            draw(cx);
            let actual = view.read_with(cx, |view, _| view.queue_width);
            assert!(
                (actual - width).abs() < px(3.),
                "preferred {width:?}, measured {actual:?}"
            );
        }
    }
    let actual = cx.debug_bounds("queue-dock").unwrap().size.width;
    assert!((actual - width).abs() < px(3.));
}

#[gpui::test]
fn returning_from_text_restores_the_active_workflow_window(cx: &mut TestAppContext) {
    use crate::app::{launch::WindowedMode, windowed_presentation::Presentation};
    cx.update(init);
    for destination in [crate::app::View::Search, crate::app::View::Help] {
        let mut state = AppState::new();
        state.view = destination;
        let model = super::surface::project(&state).unwrap();
        let mut initial = snapshot();
        initial.separate_player = true;
        initial.presentation = Presentation::windowed();
        let (engine, _updates, _commands) = Engine::fixture(initial.clone());
        let windows = WindowRegistry::default();
        let registry = windows.clone();
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                registry,
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.surface = Some(model.clone());
        next.surfaces = std::sync::Arc::new(vec![model.clone()]);
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        draw(cx);
        let handle = windows.workflows.borrow()[&model.id];
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        let text = next.presentation.request(WindowedMode::Text).unwrap();
        assert!(next.presentation.commit(text));
        view.update(cx, |view, _| {
            view.snapshot.presentation = next.presentation.clone()
        });
        let engine = view.read_with(cx, |view, _| view.engine.clone());
        let (player_window, player) = cx
            .update(|_, cx| {
                super::open_desktop_window(engine, windows.clone(), WindowRole::NowPlaying, cx)
            })
            .unwrap();
        player.update(cx, |player, _| player.snapshot = next.clone());
        let gui = next.presentation.request(WindowedMode::Gui).unwrap();
        assert!(next.presentation.commit(gui));
        next.library_window_request += 1;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        draw(cx);
        let mut late_player = view.read_with(cx, |view, _| view.snapshot.clone());
        late_player.player_window_request += 1;
        cx.update(|_, cx| {
            player_window
                .update(cx, |_, window, cx| {
                    player.update(cx, |player, cx| {
                        player.apply_snapshot(late_player, window, cx)
                    })
                })
                .unwrap();
        });
        assert_eq!(
            cx.update(|_, cx| cx.active_window())
                .map(|window| window.window_id()),
            Some(handle.window_id()),
            "The active {destination:?} window must regain the foreground"
        );
        cx.update(|_, cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            let _ = player_window.update(cx, |_, window, _| window.remove_window());
        });
    }
}

#[gpui::test]
fn handoff_reveals_graphical_panels_at_their_final_geometry(cx: &mut TestAppContext) {
    use crate::app::{launch::WindowedMode, windowed_presentation::Presentation};
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(false);
    });
    let mut initial = snapshot();
    initial.queue_open = false;
    initial.presentation = Presentation::windowed();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    for _ in 0..4 {
        draw(cx);
    }
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    let text = next.presentation.request(WindowedMode::Text).unwrap();
    assert!(next.presentation.commit(text));
    view.update(cx, |view, _| {
        view.snapshot.presentation = next.presentation.clone()
    });
    let gui = next.presentation.request(WindowedMode::Gui).unwrap();
    assert!(next.presentation.commit(gui));
    next.queue_open = true;
    next.player_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    draw(cx);
    assert!(
        (cx.debug_bounds("queue-dock").unwrap().size.width
            - view.read_with(cx, |view, _| view.queue_width))
        .abs()
            < px(3.)
    );
    assert!(view.read_with(cx, |view, _| view.column_motion_until.is_none()));
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.queue_open = false;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    draw(cx);
    assert!(
        cx.debug_bounds("queue-dock").unwrap().size.width > px(super::motion::SIDEBAR_RAIL_WIDTH)
    );
}

#[gpui::test]
fn history_track_context_menu_queues_the_clicked_occurrence(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true)
    });
    let mut initial = snapshot();
    initial.queue_open = true;
    initial.showing_queue_history = true;
    let track = crate::library::models::Track::from_folder("fixture", "album/song.mp3");
    let plays = vec![
        crate::app::listening_history::Play::from_track(&track),
        crate::app::listening_history::Play::from_track(&track),
    ];
    let target = plays[1].id.clone();
    initial.listening_history = plays.into();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut desktop = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            desktop.snapshot = initial;
            desktop
        });
        gpui_kit::base::Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..5 {
        draw(cx)
    }
    let row = cx.debug_bounds("history-track-1").unwrap();
    cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
    cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Default::default());
    draw(cx);
    cx.simulate_keystrokes("down down enter");
    draw(cx);
    assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|c|matches!(c,Command::Action(crate::app::Action::Queue(crate::app::action::QueueAction::HistoryTrack(id,crate::app::track_context::ContextKind::PlayNextInQueue))) if id==target)));
}
#[gpui::test]
fn queue_tools_stay_in_the_right_sidebar(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    initial.player.active = true;
    initial.queue_open = true;
    let mut state = AppState::new();
    state.queue.tracks = vec![Default::default()].into();
    initial.queue_tools = crate::app::queue_controls::definitions(&state);
    initial.listening_history = vec![crate::app::listening_history::Play::from_track(
        &state.queue.tracks[0],
    )]
    .into();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.presentation = WindowRole::NowPlaying;
        view.snapshot = initial;
        view
    });
    cx.simulate_resize(gpui::size(px(1200.), px(900.)));
    for _ in 0..6 {
        draw(cx);
    }
    let transport = cx.debug_bounds("transport-bar").unwrap();
    let tools = cx.debug_bounds("queue-tools").unwrap();
    let mode = cx.debug_bounds("queue-mode").unwrap();
    assert!(mode.bottom() <= tools.top());
    assert_eq!(tools.center().x, mode.center().x);
    assert!(tools.bottom() <= transport.top());
    let clear = cx.debug_bounds("queue-clear").unwrap();
    assert!(clear.top() >= tools.bottom() && clear.bottom() <= transport.top());
    for history in [false, true] {
        view.update(cx, |view, cx| {
            view.snapshot.showing_queue_history = history;
            cx.notify();
        });
        draw(cx);
        while commands.try_recv().is_ok() {}
        cx.simulate_click(clear.center(), Default::default());
        let actions = std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|cmd| match cmd {
                Command::Action(action) => Some(action),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            (history, actions.as_slice()),
            (
                false,
                [crate::app::Action::Queue(
                    crate::app::action::QueueAction::ClearQueue
                )]
            ) | (
                true,
                [crate::app::Action::Queue(
                    crate::app::action::QueueAction::ClearHistory
                )]
            )
        ));
    }
    let row = cx
        .debug_bounds("history-track-0")
        .expect("visible History track");
    cx.simulate_click(row.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Action(crate::app::Action::Queue(
                crate::app::action::QueueAction::SelectHistory(_)
            ))
        ))
    );
    let dock = cx.debug_bounds("queue-dock").unwrap();
    let next = cx.debug_bounds("queue-rail-false").unwrap();
    let history = cx.debug_bounds("queue-rail-true").unwrap();
    assert!(next.bottom() <= history.top());
    assert_eq!(next.center().x, history.center().x);
    assert!(cx.debug_bounds("queue-heading").unwrap().right() < next.left());
    cx.simulate_click(next.center(), Default::default());
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
        Command::GuardedActions { actions, .. } if matches!(actions.as_slice(), [crate::app::Action::Queue(crate::app::action::QueueAction::ShowHistory(false))]))));
    assert!(dock.bottom() <= transport.top());
    assert!(mode.left() >= dock.left() && tools.right() <= dock.right());

    assert!(cx.debug_bounds("play-queue-tools").is_none());
    let info = cx.debug_bounds("transport-info").unwrap();
    assert!(
        info.size.width >= px(480.),
        "information field must be wide: {info:?}"
    );
    assert_eq!(transport.size.height, px(super::motion::TRANSPORT_HEIGHT));
    let seek = cx.debug_bounds("transport-seek-row").unwrap();
    assert!(seek.bottom() <= info.bottom());
    cx.simulate_click(history.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|cmd| matches!(cmd, Command::ToggleQueue))
    );
    assert!(cx.debug_bounds("collapse-player-sheet").is_some());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.player.active = true;
    next.player_window_request += 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert_eq!(cx.debug_bounds("transport-info").unwrap().size, info.size);
    let collapse = cx.debug_bounds("collapse-player-sheet").unwrap();
    assert!(collapse.right() <= cx.debug_bounds("workspace-content").unwrap().right());
    assert_eq!(
        cx.debug_bounds("transport-info").unwrap().left(),
        info.left()
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.queue_open = false;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..6 {
        draw(cx);
    }
    assert_eq!(cx.debug_bounds("transport-info").unwrap().size, info.size);
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.queue_open = true;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    cx.simulate_resize(gpui::size(px(832.), px(540.)));
    for _ in 0..6 {
        draw(cx);
    }
    let row = cx.debug_bounds("visualizer-mode-row").unwrap();
    let expand = cx.debug_bounds("expand-visualizer").unwrap();
    assert!(
        expand.right() <= row.right() && expand.left() >= row.left(),
        "visualizer controls must fit: {row:?}, {expand:?}"
    );
}

#[gpui::test]
fn palette_reopens_after_dismissal_in_library_and_now_playing(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut state = AppState::new();
    crate::app::command_palette::open_with_query(&mut state, "");
    let model = super::surface::project(&state).unwrap();
    let initial = snapshot();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let saved = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        saved.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    for round in 0..4 {
        let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
        next.surface = Some(model.clone());
        next.surface_request += 1;
        next.player.active = round % 2 == 1;
        next.player_window_request += 1;
        cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
        for _ in 0..4 {
            draw(cx);
        }
        cx.update(|window, cx| assert!(window.has_active_dialog(cx), "round {round}"));
        if let (Some(row), Some(shortcut)) = (
            cx.debug_bounds("palette-command-row"),
            cx.debug_bounds("palette-shortcut"),
        ) {
            assert!(shortcut.right() <= row.right() + px(1.));
            let host = cx.debug_bounds("palette-content").unwrap();
            assert!(
                shortcut.right() <= host.right() - px(20.),
                "shortcut {shortcut:?}, host {host:?}"
            );
        }
        cx.update(|window, cx| window.close_dialog(cx));
        for _ in 0..4 {
            draw(cx);
        }
        while commands.try_recv().is_ok() {}
        cx.update(|window, cx| view.update(cx, |view, cx| view.open_palette(window, cx)));
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok())
                .any(|c| matches!(c, Command::OpenPalette { .. })),
            "dismissed palette must reopen, round {round}"
        );
    }
}

#[gpui::test]
fn deselected_rows_keep_an_opaque_background(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    let rows = std::sync::Arc::make_mut(&mut initial.columns[0].rows);
    let mut second = rows[0].clone();
    second.key = "b".into();
    second.title = "Artist B".into();
    rows.push(second);
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..4 {
        draw(cx);
    }
    let click = cx.debug_bounds("browser-row-Artist B").unwrap().center();
    cx.simulate_click(click, Default::default());
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.columns[0].selected = 1;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    for _ in 0..3 {
        draw(cx);
        let bounds = cx.debug_bounds("browser-row-Artist A").unwrap();
        cx.update(|window, cx| {
            let scale = window.scale_factor();
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(
                        |quad| (quad.bounds.origin.y.0 - f32::from(bounds.origin.y) * scale).abs()
                            < 1.
                            && (quad.bounds.size.height.0 - f32::from(bounds.size.height) * scale)
                                .abs()
                                < 1.
                            && quad.background.as_solid() == Some(cx.theme().background)
                    ),
                "A deselected row must paint its own background"
            );
        });
    }
}

#[gpui::test]
fn about_logo_scales_without_wrapping_at_narrow_widths(cx: &mut TestAppContext) {
    struct About;
    impl gpui::Render for About {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use super::*;
            div().size_full().p_4().child(about::content(cx))
        }
    }
    cx.update(init);
    let (_, cx) = cx.add_window_view(|_, _| About);
    for width in [260., 420., 900.] {
        cx.simulate_resize(gpui::size(px(width), px(700.)));
        draw(cx);
        let logo = cx.debug_bounds("about-ansi-logo").unwrap();
        assert!(logo.size.width > px(0.));
        assert!(logo.right() <= px(width - 16.), "{logo:?}");
        assert!(logo.size.width <= px(480.));
        assert!(logo.size.height < logo.size.width / 2.);
    }
}

#[gpui::test]
fn column_header_filter_expands_inline_without_scroll_reveal(cx: &mut TestAppContext) {
    cx.update(init);
    let initial = snapshot();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for _ in 0..3 {
        draw(cx);
    }
    assert!(cx.debug_bounds("column-filter-0").is_none());
    assert!(cx.debug_bounds("column-filter-toggle-0").is_none());
    let toggle = cx.debug_bounds("column-heading-0-full").unwrap();
    cx.simulate_click(toggle.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::ToggleColumnFilter { column: 0, .. }))
    );
    let mut next = view.read_with(cx, |view, _| view.snapshot.clone());
    next.filter.active = true;
    next.filter.column = 0;
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(next, window, cx)));
    draw(cx);
    let filter = cx.debug_bounds("column-filter-0").unwrap();
    assert_eq!(filter.size.height, px(28.));
    let row = cx.debug_bounds("browser-row-Artist A").unwrap();
    assert!(row.top() >= filter.bottom());
    cx.simulate_event(gpui::ScrollEvent {
        position: row.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(150.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    draw(cx);
    assert_eq!(cx.debug_bounds("column-filter-0").unwrap(), filter);
}

#[gpui::test]
fn transport_background_toggles_workspace_and_controls_handle_their_own_clicks(
    cx: &mut TestAppContext,
) {
    use crate::app::{Action, action::PlaybackAction, state::PaletteCommandKind};
    cx.update(|cx| {
        init(cx);
        cx.bind_keys([gpui::KeyBinding::new(
            "space",
            super::PlayPause,
            Some("Textamp && !Input"),
        )]);
        cx.set_reduce_motion(true);
    });
    let mut initial = snapshot();
    initial.seekable = true;
    initial.playback_id = 1;
    initial.duration_ms = 170_000;
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        captured.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(px(1200.), px(900.)));
    for active in [false, true] {
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.snapshot.player.active = active;
                cx.notify();
            })
        });
        for _ in 0..6 {
            draw(cx);
        }

        while commands.try_recv().is_ok() {}
        let bar = cx.debug_bounds("transport-bar").unwrap();
        cx.simulate_click(
            gpui::point(bar.left() + px(6.), bar.top() + px(6.)),
            Default::default(),
        );
        let clicks: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(matches!(
            (active, clicks.as_slice()),
            (
                false,
                [Command::Palette(PaletteCommandKind::GotoNowPlaying)]
            ) | (true, [Command::Palette(PaletteCommandKind::GotoLibrary)])
        ));
        let play = cx.debug_bounds("transport-play").unwrap();
        cx.simulate_click(play.center(), Default::default());
        let clicks: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(matches!(
            clicks.as_slice(),
            [Command::Action(Action::Playback(
                PlaybackAction::TogglePlayPause
            ))]
        ));
        let seek = cx.debug_bounds("transport-seek-row").unwrap();
        cx.simulate_click(
            gpui::point(
                seek.left() + seek.size.width * if active { 0.75 } else { 0.5 },
                seek.center().y,
            ),
            Default::default(),
        );
        draw(cx);
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(
            !sent.iter().any(|c| matches!(c, Command::Palette(_))),
            "seek changed view"
        );
        assert!(
            sent.iter().any(
                |c| matches!(c, Command::Seek { playback_id: 1, fraction } if *fraction > 0.3)
            ),
            "seek click did not reach playback"
        );
        cx.simulate_keystrokes("space");
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
                c,
                Command::Action(Action::Playback(PlaybackAction::TogglePlayPause))
            ))
        );
    }
}

#[gpui::test]
fn initial_frame_uses_saved_theme_before_snapshot_updates(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme;
    cx.update(init);
    let mut initial = snapshot();
    initial.theme = crate::app::theme::ThemeName::Dracula;
    let expected = super::theme::colors(initial.theme).background;
    let (engine, _updates, _commands) = Engine::fixture(initial);
    let (view, cx) = cx.add_window_view(move |window, cx| {
        Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        )
    });
    draw(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.applied_theme),
        Some(crate::app::theme::ThemeName::Dracula)
    );
    assert_eq!(cx.update(|_, cx| cx.theme().background), expected);
}

#[gpui::test]
fn aluminum_rim_insets_workspace_above_full_width_transport(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.theme = crate::app::theme::ThemeName::BrushedAluminum;
    let (engine, _updates, _commands) = Engine::fixture(initial);
    let (_, cx) = cx.add_window_view(move |window, cx| {
        Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        )
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    draw(cx);
    let rim = cx.debug_bounds("aluminum-workspace-rim").unwrap();
    let transport = cx.debug_bounds("transport-bar").unwrap();
    assert_eq!(rim.left() - transport.left(), px(6.));
    assert_eq!(transport.right() - rim.right(), px(6.));
    assert!(rim.bottom() <= transport.top());
}

#[gpui::test]
fn column_filter_keeps_child_rows_and_edits_its_own_column(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    let mut child = initial.columns[0].clone();
    child.identity = "child-albums".into();
    child.rows = std::sync::Arc::new(vec![browser::Row {
        download_action: None,
        download_id: None,
        download_status: None,
        year: None,
        key: "album".into(),
        title: "Unrelated Album Title".into(),
        kind: browser::Kind::Album,
    }]);
    initial.columns.push(child);
    initial.focused_column = 1;
    initial.filter.active = true;
    initial.filter.column = 0;
    initial.filter.query = "Artist A".into();
    initial.filter.results = Some(crate::app::state::ListFilterResults {
        matched_indices: vec![0],
        total_matches: 1,
        has_more: false,
    });
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let captured = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        captured.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    for _ in 0..4 {
        draw(cx);
    }
    assert!(
        cx.debug_bounds("browser-row-Unrelated Album Title")
            .is_some()
    );
    while commands.try_recv().is_ok() {}
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.filter_input
                .update(cx, |input, cx| input.set_value("Artist B", window, cx));
            view.filter_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        })
    });
    draw(cx);
    assert!(std::iter::from_fn(||commands.try_recv().ok()).any(|command| matches!(command, Command::ColumnFilter {column:0,query,..} if query == "Artist B")));
    cx.simulate_keystrokes("enter");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Navigate {
                column_delta: 1,
                row_delta: 0,
                ..
            }
        ))
    );
    draw(cx);
    let close = cx
        .debug_bounds("close-column-filter-0")
        .expect("close filter button");
    let input = cx.debug_bounds("column-filter-0").unwrap();
    assert!(close.left() >= input.left() && close.right() <= input.right());
    cx.simulate_click(close.center(), Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Action(crate::app::Action::Search(
                crate::app::action::SearchAction::DeactivateListFilter
            ))
        ))
    );
    let mut closed = view.read_with(cx, |view, _| view.snapshot.clone());
    closed.filter = Default::default();
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(closed, window, cx)));
    draw(cx);
    assert!(cx.debug_bounds("column-filter-0").is_none());
    assert!(cx.debug_bounds("column-heading-0-full").is_some());
}

#[gpui::test]
fn similar_album_click_opens_destination_album_details(cx: &mut TestAppContext) {
    similar_click_opens_library_destination(cx, true);
}

#[gpui::test]
fn similar_track_click_requests_a_guarded_library_jump(cx: &mut TestAppContext) {
    similar_click_opens_library_destination(cx, false);
}

fn similar_click_opens_library_destination(cx: &mut TestAppContext, albums: bool) {
    cx.update(init);
    let mut initial = snapshot();
    initial.preview = Some(catalog::Preview {
        title: "Source Album".into(),
        track: Some(crate::library::models::Track {
            rating_key: "source-track".into(),
            ..Default::default()
        }),
        similar: Some(crate::app::details::Similar {
            albums,
            tracks: vec![crate::library::models::Track {
                rating_key: "destination-track".into(),
                parent_rating_key: Some("destination-album".into()),
                parent_title: Some("Destination Album".into()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    });
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let engine = std::sync::Arc::new(engine);
    let (_view, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            engine,
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    draw(cx);
    draw(cx);
    let row = cx
        .debug_bounds("similar-track-0")
        .expect("similar album row");
    cx.simulate_click(row.center(), Default::default());
    let mut opened = false;
    while let Ok(command) = commands.try_recv() {
        if let Command::GuardedActions { actions, .. } = command {
            opened |= if albums {
                matches!(actions.as_slice(), [crate::app::Action::Browse(crate::app::action::BrowseAction::OpenDetailsInLibrary { track, destination: crate::app::library_details::Destination::Album })] if track.parent_rating_key.as_deref() == Some("destination-album"))
            } else {
                matches!(actions.as_slice(), [crate::app::Action::Browse(crate::app::action::BrowseAction::OpenSimilarTrack { seed_key, track_key })] if seed_key == "source-track" && track_key == "destination-track")
            };
        }
    }
    assert!(
        opened,
        "related music must open its own library destination"
    );
}

#[gpui::test]
fn search_dialog_return_submits_without_dismissing(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    cx.update(init);
    let mut state = AppState::new();
    state.popups.set_search_active(true);
    state.search.query = "song".into();
    let model = super::surface::project(&state).unwrap();
    let (engine, _updates, mut commands) = Engine::fixture(snapshot());
    let engine = std::sync::Arc::new(engine);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let saved = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            Desktop::new(
                engine,
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            )
        });
        saved.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    cx.update(|window, cx| {
        let engine = view.read(cx).engine.clone();
        super::surface_view::open_overlay(
            view.downgrade(),
            model,
            engine.commands.clone(),
            engine.snapshot.clone(),
            window,
            cx,
        );
    });
    draw(cx);
    draw(cx);
    while commands.try_recv().is_ok() {}
    for _ in 0..3 {
        cx.simulate_keystrokes("enter");
        draw(cx);
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    }
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        !sent
            .iter()
            .any(|command| matches!(command, Command::CloseSurface(_)))
    );
    assert_eq!(sent.iter().filter(|command| matches!(command, Command::Surface { operation: super::surface::Operation::Actions(actions), .. } if matches!(actions.as_slice(), [crate::app::Action::Search(crate::app::action::SearchAction::SubmitSearch)]))).count(), 3);
}

#[gpui::test]
fn closing_the_final_column_leaves_an_empty_reopenable_workspace(cx: &mut TestAppContext) {
    cx.update(init);
    let initial = snapshot();
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    draw(cx);
    draw(cx);
    let close = cx.debug_bounds("close-column-0").unwrap().center();
    cx.simulate_click(close, Default::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::CloseColumn { column: 0, .. }))
    );
    let mut empty = view.read_with(cx, |view, _| view.snapshot.clone());
    empty.columns.clear();
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(empty, window, cx)));
    draw(cx);
    view.read_with(cx, |view, _| {
        assert!(view.snapshot.columns.is_empty());
        assert_eq!(view.column_reveal, None);
    });
    assert!(cx.debug_bounds("close-column-0").is_none());
    cx.update(|window, cx| view.update(cx, |view, cx| view.apply_snapshot(snapshot(), window, cx)));
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("close-column-0").is_some());
}

#[gpui::test]
fn closing_a_sheet_column_returns_to_the_sidebar_root(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.sheet_browser = true;
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop.sheet_sidebar_root = false;
        desktop
    });
    cx.simulate_resize(gpui::size(px(1200.), px(800.)));
    for _ in 0..10 {
        draw(cx);
    }
    let close = cx.debug_bounds("close-column-0").unwrap().center();
    cx.simulate_click(close, Default::default());
    draw(cx);
    assert!(view.read_with(cx, |view, _| view.sheet_sidebar_root && view.sheet_open()));
    assert!(cx.debug_bounds("library-empty-wordmark").is_none());
    assert!(cx.debug_bounds("close-column-0").is_none());
}

#[gpui::test]
fn single_window_settings_uses_a_dialog_without_replacing_library(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    use gpui_kit::{ParentElement, Styled};
    cx.update(|cx| {
        init(cx);
        super::keyboard::init(cx);
    });
    cx.update(|cx| {
        cx.bind_keys([gpui_kit::KeyBinding::new(
            "cmd-w",
            super::settings::CloseSettings,
            Some("TextampSettings"),
        )])
    });
    let initial = snapshot();
    let (engine, _updates, mut commands) = Engine::fixture(initial);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let saved = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let registry = WindowRegistry::default();
        registry.borrow_mut()[WindowRole::Library.index()] = Some(window.window_handle());
        let view = cx.new(|cx| {
            Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                registry,
                window,
                cx,
            )
        });
        saved.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = slot.borrow().clone().unwrap();
    draw(cx);
    let previous_focus = cx.update(|window, cx| {
        let focus = view.read(cx).volume_focus.clone();
        focus.focus(window, cx);
        focus
    });
    view.update(cx, |view, cx| view.open_settings(cx));
    draw(cx);
    draw(cx);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    let dialog_focus = cx.update(|window, cx| window.focused(cx));
    cx.simulate_keystrokes("tab");
    draw(cx);
    assert_ne!(cx.update(|window, cx| window.focused(cx)), dialog_focus);
    assert!(!cx.update(|window, _| previous_focus.is_focused(window)));
    view.update(cx, |view, cx| view.open_settings(cx));
    draw(cx);
    cx.simulate_keystrokes("cmd-w");
    draw(cx);
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    cx.run_until_parked();
    draw(cx);
    draw(cx);
    assert!(cx.update(|window, _| previous_focus.is_focused(window)));
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::SettingsClosed))
    );
    cx.update(|window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog.title("Search").child(gpui::div().h(px(120.)))
        })
    });
    draw(cx);
    view.update(cx, |view, cx| view.open_settings(cx));
    draw(cx);
    assert!(
        cx.debug_bounds("settings-content").is_some(),
        "Settings must reopen even when another dialog is present"
    );
    cx.simulate_resize(gpui::size(px(600.), px(580.)));
    draw(cx);
    let content = cx.debug_bounds("settings-content").unwrap();
    assert!(content.size.width <= px(600.) && content.size.height <= px(580.));
}

#[gpui::test]
fn modal_backdrop_preserves_titlebar_input_and_blocks_library_input(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt;
    use gpui_kit::{InteractiveElement, IntoElement, ParentElement, Styled};
    struct PointerProbe(std::rc::Rc<std::cell::Cell<usize>>);
    impl gpui::Render for PointerProbe {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let presses = self.0.clone();
            gpui::div()
                .size_full()
                .on_mouse_down(gpui::MouseButton::Left, move |_, _, _| {
                    presses.set(presses.get() + 1)
                })
        }
    }
    cx.update(init);
    let presses = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = presses.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let content = cx.new(|_| PointerProbe(counter));
        gpui_kit::base::Root::new(content, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.), px(700.)));
    cx.update(|window, cx| {
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .title("Settings")
                .overlay_closable(false)
                .w(px(500.))
                .margin_top(px(60.))
                .child(gpui::div().h(px(300.)))
        })
    });
    draw(cx);
    draw(cx);
    cx.simulate_mouse_down(
        gpui::point(px(750.), px(17.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    cx.simulate_mouse_up(
        gpui::point(px(750.), px(17.)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    assert_eq!(
        presses.get(),
        1,
        "title-bar presses must reach the window chrome"
    );
    cx.simulate_click(gpui::point(px(750.), px(500.)), Default::default());
    assert_eq!(
        presses.get(),
        1,
        "the library must remain modal below the title bar"
    );
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
}

#[gpui::test]
fn entering_multiple_windows_creates_a_detached_player_and_keeps_the_library(
    cx: &mut TestAppContext,
) {
    cx.update(init);
    let mut initial = snapshot();
    initial.presentation = crate::app::windowed_presentation::Presentation::windowed();
    let (engine, updates, _commands) = Engine::fixture(initial.clone());
    let registry = WindowRegistry::default();
    let windows = registry.clone();
    let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = saved.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let desktop = cx.new(|cx| {
            Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                windows,
                window,
                cx,
            )
        });
        slot.replace(Some(desktop.clone()));
        gpui_kit::base::Root::new(desktop, window, cx)
    });
    registry.borrow_mut()[WindowRole::Library.index()] =
        Some(cx.update(|window, _| window.window_handle()));
    let desktop = saved.borrow().clone().unwrap();
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.apply_snapshot(initial.clone(), window, cx)
        })
    });
    draw(cx);
    assert!(registry.borrow()[WindowRole::NowPlaying.index()].is_none());
    let mut multiple = initial;
    multiple.separate_player = true;
    updates.send_replace(multiple.clone());
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.apply_snapshot(multiple, window, cx)
        })
    });
    cx.run_until_parked();
    draw(cx);
    draw(cx);
    let player = registry.borrow()[WindowRole::NowPlaying.index()]
        .expect("Multiple Windows must create a player window");
    let visualizer = registry.borrow()[WindowRole::Visualizer.index()].unwrap();
    let library = registry.borrow()[WindowRole::Library.index()].unwrap();
    assert_ne!(visualizer.window_id(), player.window_id());
    assert_ne!(visualizer.window_id(), library.window_id());
    cx.update(|_, cx| {
        let size = player
            .update(cx, |_, window, _| window.viewport_size())
            .unwrap();
        assert_eq!(size, gpui::size(px(840.), px(550.)));
    });
    assert_ne!(library.window_id(), player.window_id());
    cx.update(|_, cx| assert!(player.update(cx, |_, _, _| ()).is_ok()));
    assert!(!desktop.read_with(cx, |desktop, _| desktop.snapshot.player.active));
    let mut requested = desktop.read_with(cx, |desktop, _| desktop.snapshot.clone());
    requested.player_window_request += 1;
    updates.send_replace(requested.clone());
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.apply_snapshot(requested, window, cx)
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(cx.windows().len(), 3));
    assert_eq!(
        registry.borrow()[WindowRole::Visualizer.index()]
            .unwrap()
            .window_id(),
        visualizer.window_id()
    );
    assert_eq!(
        registry.borrow()[WindowRole::NowPlaying.index()]
            .unwrap()
            .window_id(),
        player.window_id()
    );
    let mut single = desktop.read_with(cx, |desktop, _| desktop.snapshot.clone());
    single.separate_player = false;
    single
        .presentation
        .set_workspace(crate::app::windowed_presentation::Workspace::NowPlaying);
    updates.send_replace(single.clone());
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| desktop.apply_snapshot(single, window, cx))
    });
    cx.run_until_parked();
    draw(cx);
    draw(cx);
    cx.update(|_, cx| {
        assert!(player.update(cx, |_, _, _| ()).is_err());
        assert!(visualizer.update(cx, |_, _, _| ()).is_err());
    });
    cx.update(|window, _| assert_eq!(window.window_handle().window_id(), library.window_id()));
    assert!(desktop.read_with(cx, |desktop, _| desktop.snapshot.player.active));
    let mut multiple = desktop.read_with(cx, |desktop, _| desktop.snapshot.clone());
    multiple.separate_player = true;
    updates.send_replace(multiple.clone());
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.apply_snapshot(multiple, window, cx)
        })
    });
    cx.run_until_parked();
    draw(cx);
    draw(cx);
    let player = registry.borrow()[WindowRole::NowPlaying.index()].unwrap();
    cx.update(|_, cx| {
        player
            .update(cx, |_, window, _| window.remove_window())
            .unwrap()
    });
    let unchanged = desktop.read_with(cx, |desktop, _| desktop.snapshot.clone());
    cx.update(|window, cx| {
        desktop.update(cx, |desktop, cx| {
            desktop.apply_snapshot(unchanged, window, cx)
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        assert!(
            player.update(cx, |_, _, _| ()).is_err(),
            "A manually closed player must stay closed until requested"
        )
    });
}

#[gpui::test]
fn multi_window_visualizer_has_its_own_controls_without_player_metadata(cx: &mut TestAppContext) {
    cx.update(init);
    for role in [WindowRole::NowPlaying, WindowRole::Visualizer] {
        let mut initial = snapshot();
        initial.separate_player = true;
        initial.player.active = true;
        initial.player.mode = 1;
        let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let mut desktop = Desktop::new(
                std::sync::Arc::new(engine),
                role,
                WindowRegistry::default(),
                window,
                cx,
            );
            desktop.snapshot = initial;
            desktop
        });
        for (width, height) in [(960., 600.), (500., 300.)] {
            cx.simulate_resize(gpui::size(px(width), px(height)));
            draw(cx);
            let visualizer = role == WindowRole::Visualizer;
            assert!(cx.debug_bounds("toggle-sidebar").is_none());
            assert_eq!(cx.debug_bounds("audio-visualizer").is_some(), visualizer);
            assert_eq!(
                cx.debug_bounds("visualizer-mode-tabs").is_some(),
                visualizer
            );
            assert!(cx.debug_bounds("expand-visualizer").is_none());
            if visualizer {
                assert!(cx.debug_bounds("now-playing-metadata").is_none());
                let tabs = cx.debug_bounds("visualizer-mode-tabs").unwrap();
                cx.simulate_click(
                    gpui::point(tabs.left() + px(140.), tabs.top() + px(12.)),
                    Default::default(),
                );
                assert!(
                    std::iter::from_fn(|| commands.try_recv().ok())
                        .any(|command| matches!(command, Command::PlayerMode(_)))
                );
            }
        }
    }
}

#[gpui::test]
fn compact_player_artist_name_remains_a_library_detail_link(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.player.track = Some(crate::library::models::Track {
        rating_key: "track".into(),
        title: "Song".into(),
        original_title: Some("Track Artist".into()),
        grandparent_title: Some("Album Artist".into()),
        ..Default::default()
    });
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut view = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        view.snapshot = initial;
        view
    });
    for height in [760., 240.] {
        cx.simulate_resize(gpui::size(px(500.), px(height)));
        draw(cx);
        let artist = cx.debug_bounds("mini-artist").unwrap();
        cx.simulate_click(artist.center(), Default::default());
        assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::Palette(crate::app::state::PaletteCommandKind::FromTrackContext {
                kind: crate::app::track_context::ContextKind::ShowDetails(crate::app::library_details::Destination::Artist), track,
            }) if track.track_artist() == "Track Artist")));
    }
}

#[gpui::test]
fn keyboard_tab_reaches_transport_controls_without_switching_workspace(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        super::keyboard::init(cx);
        cx.bind_keys([
            gpui::KeyBinding::new(
                "down",
                super::MoveDown,
                Some("Textamp && !TextampSlider && !Input"),
            ),
            gpui::KeyBinding::new(
                "enter",
                super::Activate,
                Some("Textamp && !TextampSlider && !Input"),
            ),
        ]);
    });
    let mut initial = snapshot();
    initial.seekable = true;
    initial.duration_ms = 120_000;
    let (engine, _updates, mut commands) = Engine::fixture(initial);
    let (view, cx) = cx.add_window_view(move |window, cx| {
        Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        )
    });
    draw(cx);
    draw(cx);
    let original = cx.update(|window, cx| window.focused(cx));
    cx.simulate_keystrokes("tab");
    draw(cx);
    let pane = cx.update(|window, cx| window.focused(cx));
    assert!(pane.is_some() && pane != original);
    cx.simulate_keystrokes("shift-tab");
    draw(cx);
    assert_ne!(cx.update(|window, cx| window.focused(cx)), pane);
    cx.simulate_keystrokes("tab");
    draw(cx);
    assert_eq!(cx.update(|window, cx| window.focused(cx)), pane);
    use super::keyboard::Pane;
    for _ in 0..12 {
        if cx.update(|window, cx| {
            view.read(cx)
                .pane_focus
                .handle(Pane::Transport)
                .contains_focused(window, cx)
        }) {
            break;
        }
        cx.simulate_keystrokes("tab");
        draw(cx);
    }
    assert!(cx.update(|window, cx| {
        view.read(cx)
            .pane_focus
            .handle(Pane::Transport)
            .contains_focused(window, cx)
    }));
    let mut seek = false;
    let mut volume = false;
    for _ in 0..25 {
        cx.update(|window, cx| {
            let view = view.read(cx);
            seek |= view.seek_focus.is_focused(window);
            volume |= view.volume_focus.is_focused(window);
        });
        if seek && volume && cx.update(|window, cx| view.read(cx).volume_focus.is_focused(window)) {
            break;
        }
        cx.simulate_keystrokes("down");
        draw(cx);
    }
    assert!(
        seek && volume,
        "both sliders must be reachable within the transport pane"
    );
    cx.simulate_keystrokes("right");
    let pending: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(pending.iter().any(|command| matches!(
        command,
        Command::Action(crate::app::Action::Playback(
            crate::app::action::PlaybackAction::VolumeUp
        ))
    )));
    assert!(!pending.iter().any(|command| matches!(
        command,
        Command::CoreKey(_)
            | Command::PlayerKey(_)
            | Command::Palette(crate::app::state::PaletteCommandKind::GotoNowPlaying)
    )));
}

#[gpui::test]
fn keyboard_queue_rail_opens_history_with_enter(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        super::keyboard::init(cx);
        cx.bind_keys([
            gpui::KeyBinding::new("down", super::MoveDown, Some("Textamp")),
            gpui::KeyBinding::new("enter", super::Activate, Some("Textamp")),
        ]);
    });
    let mut initial = snapshot();
    initial.queue_open = false;
    let (engine, _updates, mut commands) = Engine::fixture(initial);
    let (view, cx) = cx.add_window_view(move |window, cx| {
        Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        )
    });
    let rail = cx.update(|_, cx| {
        view.read(cx)
            .pane_focus
            .handle(super::keyboard::Pane::Rail)
            .clone()
    });
    for _ in 0..12 {
        draw(cx);
        if cx.update(|window, cx| rail.contains_focused(window, cx)) {
            break;
        }
        cx.simulate_keystrokes("tab");
    }
    assert!(cx.update(|window, cx| rail.contains_focused(window, cx)));
    cx.simulate_keystrokes("down");
    draw(cx);
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| {
        use gpui::InputEvent as _;
        window.dispatch_event(
            gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("enter").unwrap(),
            }
            .to_platform_input(),
            cx,
        );
    });
    draw(cx);
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| {
            matches!(command, Command::GuardedActions { actions, .. }
            if actions.iter().any(|action| matches!(action,
                crate::app::Action::Queue(crate::app::action::QueueAction::ShowHistory(true)))))
        })
    );
}

#[gpui::test]
fn holding_play_opens_choices_without_starting_playback(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    let track = crate::library::models::Track {
        rating_key: "track".into(),
        ..Default::default()
    };
    initial.preview = Some(catalog::Preview {
        controls: catalog::compact_controls(crate::app::playback_controls::track_controls(
            &track, None,
        )),
        track: Some(track),
        ..Default::default()
    });
    let (engine, _, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        gpui_kit::base::Root::new(view, window, cx)
    });
    draw(cx);
    draw(cx);
    let button = cx.debug_bounds("details-control-0").expect("Play control");
    while commands.try_recv().is_ok() {}
    cx.simulate_mouse_down(button.center(), gpui::MouseButton::Left, Default::default());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(550));
    cx.run_until_parked();
    draw(cx);
    cx.simulate_mouse_up(button.center(), gpui::MouseButton::Left, Default::default());
    draw(cx);
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, Command::GuardedActions { .. }))
    );
    cx.simulate_keystrokes("down down enter");
    draw(cx);
    assert!(std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command, Command::GuardedActions {actions,..} if matches!(actions.as_slice(), [crate::app::Action::Queue(crate::app::action::QueueAction::EnqueueTracksNext(_))]))));
}

#[gpui::test]
fn radio_queue_mode_selector_dispatches_manual_mode_from_sidebar(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let mut state = AppState::new();
    state.playback_mode = crate::app::state::PlaybackMode::Radio;
    state.radio.active_station = Some(crate::app::state::ActiveStation {
        source: crate::library::models::RadioSource::Station("nav-radio/randomAlbum".into()),
        title: "Play Random Albums".into(),
    });
    let mut initial = snapshot();
    initial.player.active = true;
    initial.queue_open = true;
    initial.queue_mode = crate::app::queue_modes::label(&state);
    initial.queue_modes = crate::app::queue_modes::choices(&state);
    let (engine, _updates, mut commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    cx.simulate_resize(gpui::size(px(1100.), px(800.)));
    for _ in 0..6 {
        draw(cx);
    }
    let button = cx
        .debug_bounds("queue-mode")
        .expect("queue mode is reachable from the sidebar");
    let rail = cx.debug_bounds("library-queue-rail").unwrap();
    assert!(button.left() >= rail.left() && button.right() <= rail.right());
    assert!(button.size.width < px(60.));
    cx.simulate_click(button.center(), Default::default());
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Palette(crate::app::state::PaletteCommandKind::StopRadio)
        ))
    );
}

#[gpui::test]
fn palette_edit_availability_uses_the_original_focus(cx: &mut TestAppContext) {
    cx.update(init);
    let mut initial = snapshot();
    initial.filter.active = true;
    initial.filter.query = "Artist".into();
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = saved.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = Desktop::new(
                std::sync::Arc::new(engine),
                WindowRole::Library,
                WindowRegistry::default(),
                window,
                cx,
            );
            view.snapshot = initial;
            view
        });
        slot.replace(Some(view.clone()));
        gpui_kit::base::Root::new(view, window, cx)
    });
    let view = saved.borrow().clone().unwrap();
    for _ in 0..3 {
        draw(cx);
    }
    let native = super::menu_catalog::native_entries(&super::menu_catalog::menus(&[]))
        .into_iter()
        .find_map(|item| match item.operation {
            super::surface::Operation::Native(native) if native.label == "Cut" => Some(native),
            _ => None,
        })
        .unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.focus.focus(window, cx);
            view.open_palette(window, cx);
        })
    });
    assert!(!cx.update(|_, cx| native.available(cx.try_global::<super::menu_catalog::Origin>())));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.filter_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx)
        })
    });
    draw(cx);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_palette(window, cx)));
    assert!(cx.update(|_, cx| native.available(cx.try_global::<super::menu_catalog::Origin>())));
    cx.update(|window, cx| view.update(cx, |view, cx| view.focus.focus(window, cx)));
    assert!(cx.update(|_, cx| native.available(cx.try_global::<super::menu_catalog::Origin>())));
}

#[gpui::test]
fn lyrics_reader_renders_a_scrollable_reading_area(cx: &mut TestAppContext) {
    cx.update(init);
    let mut state = AppState::new();
    let track =
        serde_json::from_value(serde_json::json!({"ratingKey":"song","title":"Fixture song"}))
            .unwrap();
    state.queue.tracks = vec![track];
    state.queue.index = Some(0);
    state.popups.set_lyrics(Some(crate::app::lyrics::Panel {
        track: state.queue.tracks[0].clone(),
        generation: state.library_generation,
        request: 1,
        playback_bound: true,
        content: crate::app::lyrics::Content::Ready(vec![crate::app::lyrics::Document {
            lang: "en".into(),
            synced: true,
            line: vec![
                crate::app::lyrics::Line {
                    value: "Opening line".into(),
                    start: Some(0),
                    end: None,
                },
                crate::app::lyrics::Line {
                    value: "Second line".into(),
                    start: Some(2000),
                    end: None,
                },
            ],
            ..Default::default()
        }]),
        language: 0,
        follow: true,
        scroll: 0,
    }));
    let model = super::surface::project(&state).unwrap();
    assert!(model.toggles[0].checked);
    let mut initial = snapshot();
    initial.player.active = true;
    initial.surface = Some(model);
    let (engine, _updates, _commands) = Engine::fixture(initial.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let mut desktop = Desktop::new(
            std::sync::Arc::new(engine),
            WindowRole::Library,
            WindowRegistry::default(),
            window,
            cx,
        );
        desktop.snapshot = initial;
        desktop
    });
    cx.simulate_resize(gpui::size(px(1300.), px(820.)));
    for _ in 0..4 {
        draw(cx);
    }
    let reading = cx.debug_bounds("lyrics-lines").expect("lyrics viewport");
    let pane = cx
        .debug_bounds("visualizer-pane")
        .expect("player detail pane");
    assert!(reading.top() >= pane.top() && reading.bottom() <= pane.bottom());

    assert!(reading.size.height > px(100.));
    let first = cx.debug_bounds("lyrics-line-0").expect("opening lyric");
    let second = cx.debug_bounds("lyrics-line-1").expect("second lyric");
    assert!(first.top() >= reading.top() && second.bottom() <= reading.bottom());
}
