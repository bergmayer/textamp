//! Connection setup uses only loopback fixtures and isolated credential stores.
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use textamp::app::dispatch::{dispatch_action, handle_core_event};
use textamp::app::sources::{
    self,
    dialogs::{Dialog, ServerForm},
    ActiveSource,
};
use textamp::app::state::{PlayStatus, SettingsFocus, SettingsSection, View};
use textamp::app::{AppState, Event};
use textamp::audio::AudioPlayer;
use textamp::config::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

async fn fixture(
    reply: impl Fn(&str, &HashMap<String, String>, &str, &Value) -> (u16, String, String)
        + Send
        + Sync
        + 'static,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let reply = Arc::new(reply);
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut chunk = [0; 4096];
                let end = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(n) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break n + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&data[..end]).into_owned();
                let len = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while data.len() < end + len {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..n]);
                }
                let url = reqwest::Url::parse(&format!(
                    "http://fixture{}",
                    headers.split_whitespace().nth(1).unwrap()
                ))
                .unwrap();
                let query = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let body = serde_json::from_slice(&data[end..]).unwrap_or(Value::Null);
                let (code, extra, body) = reply(url.path(), &query, &headers, &body);
                let response = format!("HTTP/1.1 {code} Fixture\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    (url, task)
}

#[test]
fn setup_uses_isolated_configuration() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "combined_setup_worker", "--nocapture"])
        .env("TEXTAMP_SETUP_FIXTURE", "1")
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CACHE_HOME", root.path().join("cache"))
        .env("XDG_STATE_HOME", root.path().join("state"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn combined_setup_worker() {
    if std::env::var("TEXTAMP_SETUP_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    for (valid, include_analysis) in [(false, true), (true, true), (true, false)] {
        let (target, server) = fixture(move |path, query, headers, body| {
            if path.starts_with("/am/") {
                assert!(include_analysis, "AudioMuse must be optional");
            }
            if path == "/am/auth" {
                assert_eq!(body["user"], "analysis-user");
                assert_eq!(body["password"], "analysis-secret");
                return (
                    if valid { 200 } else { 401 },
                    "Set-Cookie: audiomuse_jwt=fixture; Path=/\r\n".into(),
                    "{}".into(),
                );
            }
            let payload = if path == "/am/api/servers" {
                json!({"servers":[{"name":"Fixture music","server_type":"navidrome"}]})
            } else if path == "/am/api/sync" {
                assert!(headers.contains("audiomuse_jwt=fixture"));
                json!({"tracks":[],"total_tracks":0,"has_more":false,"provider_type":"navidrome"})
            } else {
                assert!(path.starts_with("/rest/"));
                if !query.is_empty() {
                    assert_eq!(query["u"], "music-user");
                }
                let mut payload = match path {
                    "/rest/getMusicFolders" => {
                        json!({"musicFolders":{"musicFolder":[{"id":"one","name":"Fixture"}]}})
                    }
                    "/rest/getArtists" => json!({"artists":{"index":[]}}),
                    "/rest/getAlbumList2" => json!({"albumList2":{"album":[]}}),
                    "/rest/search3" => json!({"searchResult3":{"song":[]}}),
                    "/rest/getPlaylists" => json!({"playlists":{"playlist":[]}}),
                    _ => json!({}),
                };
                payload["status"] = json!("ok");
                json!({"subsonic-response":payload})
            };
            (200, String::new(), payload.to_string())
        })
        .await;
        let target_copy = target.clone();
        let (alias, proxy) = fixture(move |path, query, headers, _| {
            assert!(
                query.is_empty(),
                "redirect resolution must not send login parameters"
            );
            assert!(!headers.to_lowercase().contains("authorization:"));
            assert!(!headers.to_lowercase().contains("cookie:"));
            let path = path
                .strip_prefix("/music")
                .map(str::to_owned)
                .or_else(|| path.strip_prefix("/analysis").map(|s| format!("/am{s}")))
                .unwrap();
            (
                302,
                format!("Location: {target_copy}{path}\r\n"),
                String::new(),
            )
        })
        .await;
        let mut state = AppState::new();
        state.view = View::Settings;
        state.settings_state.section = SettingsSection::Libraries;
        state.settings_state.focus = SettingsFocus::Content;
        state.sources.active = ActiveSource::Folder("keep-playing".into());
        state.playback.status = PlayStatus::Paused;
        state.playback.position_ms = 12345;
        let mut form = ServerForm::new(None);
        let id = form.source.id.clone();
        for (field, value) in form.fields.iter_mut().zip([
            format!("{}/music", alias.trim_start_matches("http://")),
            "music-user".into(),
            "music-secret".into(),
            "Fixture".into(),
            if include_analysis {
                format!("{alias}/analysis")
            } else {
                String::new()
            },
            "analysis-user".into(),
            "analysis-secret".into(),
            "Fixture music".into(),
        ]) {
            field.value = value.into();
            field.cursor = field.value.len();
        }
        state
            .popups
            .set_library_dialog(Some(Dialog::Navidrome(form)));
        let actions = sources::dialogs::key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ),
            &mut state,
        );
        assert_eq!(actions.len(), 1);
        let mut config = Config::default();
        let mut audio = AudioPlayer::new_without_audio();
        let (tx, mut rx) = mpsc::channel::<Event>(32);
        for action in actions {
            dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                .await
                .unwrap();
        }
        assert!(config.navidrome_sources.is_empty());
        while matches!(state.popups.library_dialog(),Some(Dialog::Navidrome(f)) if f.busy) {
            let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            for action in handle_core_event(event, &mut state, &tx) {
                dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(
            state.sources.active.folder().map(String::as_str),
            Some("keep-playing")
        );
        assert_eq!(state.playback.position_ms, 12345);
        if valid {
            assert_eq!(config.navidrome_sources[0].url, target);
            assert!(state
                .settings_state
                .cache_scans
                .values()
                .any(|s| matches!(s.work, sources::cache::Work::Scan(_))));
            if include_analysis {
                let (key, connection) = config.audiomuse_connections.iter().next().unwrap();
                assert_eq!(connection.url, format!("{target}/am"));
                assert_eq!(
                    textamp::library::credentials::load_source("audiomuse", key)
                        .unwrap()
                        .unwrap()
                        .as_str(),
                    "analysis-secret"
                );
                let choice = sources::library_choices(&state)
                    .into_iter()
                    .find(|c| matches!(c, sources::LibraryChoice::Navidrome { .. }))
                    .unwrap();
                let form = ServerForm::new(Some(&config.navidrome_sources[0]))
                    .with_analysis(&choice, &state);
                assert_eq!(form.fields[4].value.as_str(), format!("{target}/am"));
                assert!(form.fields[6].value.is_empty());
                // Editing AudioMuse alone resolves the alias too, retaining its
                // private password when the user leaves the password field blank.
                let key = key.clone();
                state.popups.set_library_dialog(Some(Dialog::AudioMuse(
                    sources::audiomuse::Form {
                        instance: uuid::Uuid::new_v4().to_string(),
                        library_key: key.clone(),
                        fields: [
                            sources::dialogs::Field {
                                value: format!("{alias}/analysis").into(),
                                ..Default::default()
                            },
                            sources::dialogs::Field {
                                value: "analysis-user".into(),
                                ..Default::default()
                            },
                            Default::default(),
                            sources::dialogs::Field {
                                value: "Fixture music".into(),
                                ..Default::default()
                            },
                        ],
                        focus: 0,
                        error: None,
                        task: None,
                    },
                )));
                dispatch_action(
                    sources::audiomuse::Command::Submit.into(),
                    &mut state,
                    &mut audio,
                    &mut config,
                    &tx,
                )
                .await
                .unwrap();
                while matches!(state.popups.library_dialog(),Some(Dialog::AudioMuse(form)) if form.task.is_some())
                {
                    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    for action in handle_core_event(event, &mut state, &tx) {
                        dispatch_action(action, &mut state, &mut audio, &mut config, &tx)
                            .await
                            .unwrap();
                    }
                }
                assert!(
                    state.popups.library_dialog().is_none(),
                    "standalone AudioMuse setup must save successfully"
                );
                assert_eq!(
                    config.audiomuse_connections[&key].url,
                    format!("{target}/am")
                );
            } else {
                assert!(config.audiomuse_connections.is_empty());
            }
        } else {
            assert!(config.navidrome_sources.is_empty());
            assert!(config.audiomuse_connections.is_empty());
            assert!(textamp::library::credentials::load_source("navidrome", &id)
                .unwrap()
                .is_none());
            assert!(
                matches!(state.popups.library_dialog(),Some(Dialog::Navidrome(form)) if form.error.is_some())
            );
        }
        state.sources.nav_tasks.clear();
        state.settings_state.cache_scans.clear();
        server.abort();
        proxy.abort();
    }
}

#[test]
fn short_addresses_are_normalized_and_credentials_do_not_belong_in_urls() {
    use textamp::util::server_url::normalize;
    assert_eq!(
        normalize("flame/music").unwrap().as_str(),
        "http://flame/music"
    );
    assert_eq!(
        normalize("https://flame/music/").unwrap().as_str(),
        "https://flame/music/"
    );
    for url in [
        "",
        "ftp://flame/music",
        "https://user:secret@flame/music",
        "https://flame/music?token=secret",
    ] {
        assert!(normalize(url).is_err());
    }
}

#[tokio::test]
async fn redirect_loops_and_website_login_redirects_do_not_become_server_addresses() {
    let (url, server) = fixture(|path, _, _, _| {
        if path.starts_with("/loop/") {
            (302, "Location: /loop/rest/ping\r\n".into(), String::new())
        } else if path == "/login" {
            (200, String::new(), "<html>sign in</html>".into())
        } else {
            (302, "Location: /login\r\n".into(), String::new())
        }
    })
    .await;
    assert!(
        textamp::util::server_url::resolve(&format!("{url}/loop"), "rest/ping")
            .await
            .is_err()
    );
    assert!(textamp::util::server_url::resolve(&url, "rest/ping")
        .await
        .unwrap_err()
        .to_string()
        .contains("website login"));
    server.abort();
}
