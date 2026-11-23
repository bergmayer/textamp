use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use textamp::app::{
    AppState,
    lyrics::{Content, Document, Line, Panel},
};
fn render(state: &mut AppState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| state.apply_render_feedback(textamp::ui::render(frame, state)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn terminal_lyrics_follow_seeks_and_plain_text_scrolls_without_following() {
    let mut state = AppState::new();
    let track =
        serde_json::from_value(serde_json::json!({"ratingKey":"song","title":"Fixture song"}))
            .unwrap();
    state.queue.tracks = vec![track];
    state.queue.index = Some(0);
    state.playback.position_ms = 25_000;
    state.popups.set_lyrics(Some(Panel {
        track: state.queue.tracks[0].clone(),
        generation: state.library_generation,
        request: 1,
        playback_bound: true,
        content: Content::Ready(vec![Document {
            synced: true,
            line: (0..40)
                .map(|index| Line {
                    value: format!("Verse {index:02}"),
                    start: Some(index * 1000),
                    end: None,
                })
                .collect(),
            ..Default::default()
        }]),
        language: 0,
        follow: true,
        scroll: 0,
    }));
    let text = render(&mut state);
    assert!(text.contains("Verse 25"));
    assert!(!text.contains("Verse 00"));
    state.playback.position_ms = 0;
    assert!(render(&mut state).contains("Verse 00"));
    if let Content::Ready(documents) = &mut state.popups.lyrics_mut().unwrap().content {
        documents[0].synced = false;
    }
    for _ in 0..15 {
        textamp::app::handlers::key_input::handle_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut state,
            &Default::default(),
        );
    }
    let text = render(&mut state);
    assert!(text.contains("Verse 15"));
    assert!(!text.contains("Verse 00"));
    textamp::app::handlers::key_input::handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut state,
        &Default::default(),
    );
    assert!(state.popups.lyrics().is_none());
}
