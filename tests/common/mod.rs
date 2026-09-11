//! Server-capable state without credentials, network requests or audio hardware.
pub fn navidrome_state() -> textamp::app::AppState {
    use textamp::app::sources::{navidrome::Session, ActiveSource};
    use textamp::navidrome::{Client, Source};
    let source = Source {
        id: "fixture".into(),
        name: "Fixture".into(),
        url: "http://127.0.0.1:1".into(),
        username: "fixture".into(),
        libraries: vec![],
    };
    let mut state = textamp::app::AppState::new();
    state.sources.active = ActiveSource::Navidrome(Box::new(Session {
        client: Client::new(&source, "fixture".into(), None).unwrap(),
        source,
        extensions: std::collections::HashSet::from(["sonicSimilarity".into()]),
    }));
    state
}
