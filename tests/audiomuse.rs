//! Read-only API contracts, model boundaries and cache round trips. No live services.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use textamp::audiomuse::{Analysis, Client, Connection, Feature, Filter, Snapshot};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Request {
    path: String,
    query: HashMap<String, String>,
    body: Value,
}
struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(reply: impl Fn(Request) -> (u16, Value) + Send + Sync + 'static) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/prefix", listener.local_addr().unwrap());
    let reply = Arc::new(reply);
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let reply = reply.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut chunk = [0; 4096];
                let end = loop {
                    let n = stream.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(i) = data.windows(4).position(|s| s == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&data[..end]).into_owned();
                let len = headers
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while data.len() < end + len {
                    let n = stream.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..n]);
                }
                let url = reqwest::Url::parse(&format!(
                    "http://local{}",
                    headers.split_whitespace().nth(1).unwrap()
                ))
                .unwrap();
                let path = url.path().strip_prefix("/prefix/").unwrap().to_owned();
                let query = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let body = if len == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&data[end..]).unwrap()
                };
                let auth = path == "auth";
                if !auth {
                    assert!(headers.contains("audiomuse_jwt=fixture-session"));
                }
                let (code, body) = reply(Request { path, query, body });
                let body = body.to_string();
                let cookie = if auth {
                    "Set-Cookie: audiomuse_jwt=fixture-session; HttpOnly; Path=/\r\n"
                } else {
                    ""
                };
                let response = format!("HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    Server { url, task }
}
fn connection(url: &str) -> Connection {
    Connection {
        url: url.into(),
        username: "listener".into(),
        server: "My Navidrome".into(),
    }
}
fn analysis(id: &str, fp: &str) -> Analysis {
    serde_json::from_value(
        json!({"id":id,"fp":fp,"tempo":120.0,"energy":0.75,"key":"C","scale":"minor",
        "mood_vector":"rock:0.7,ambient:0.4","other_features":"happy:0,sad:0"}),
    )
    .unwrap()
}
fn page(tracks: Value) -> Value {
    json!({"total_tracks":tracks.as_array().unwrap().len(),"tracks":tracks,"provider_type":"navidrome","has_more":false,"next_page":null})
}

#[test]
fn labels_are_scored_descriptors_not_zero_filled_moods_or_probabilities() {
    let mut track = analysis("one", "a");
    track.mood_vector =
        Some("rock:0.7,ROCK:0.3,ambient:0.4,bad:NaN,unknown:-1,bad2:2,invalid,zero:0".into());
    assert_eq!(
        track.labels(),
        BTreeMap::from([("ambient".into(), 0.4), ("rock".into(), 0.7)])
    );
    assert!(Filter::Tempo(2).matches(&track));
    assert!(!Filter::Tempo(1).matches(&track));
    assert!(Filter::Energy(2).matches(&track));
    assert!(Filter::Key("C minor".into()).matches(&track));
    assert!(!Filter::Label("happy".into()).matches(&track));
    assert!(track.summary().contains("not tag values or probabilities"));
}

#[tokio::test]
async fn sonic_queries_are_read_only_scoped_and_validate_response_shapes() {
    let s = server(|r| {
        if r.path == "auth" {
            return (200, json!({}));
        }
        assert_eq!(r.query["server"], "My Navidrome");
        assert!(
            r.body.is_null(),
            "Discovery must not create playlists or analysis jobs"
        );
        match r.path.as_str() {
            "api/mood_centroids" => (200, json!({"calm":[{"index":3}],"empty":[]})),
            "api/similar_tracks" => {
                assert_eq!(r.query["n"], "100");
                if r.query.contains_key("mood") {
                    assert_eq!(r.query["mood"], "calm");
                    assert_eq!(r.query["centroid_index"], "3");
                } else {
                    assert_eq!(r.query["item_id"], "seed");
                }
                (200, json!([{"item_id":"match"}]))
            }
            "api/find_path" => {
                assert_eq!(r.query["start_song_id"], "seed");
                assert_eq!(r.query["max_steps"], "5");
                assert_eq!(r.query["path_space"], "audio");
                assert_eq!(r.query["path_fix_size"], "false");
                (
                    200,
                    json!({"path":[{"item_id":"seed"},{"item_id":"match"},{"item_id":"end"}]}),
                )
            }
            _ => panic!("unexpected endpoint: {}", r.path),
        }
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    assert_eq!(api.similar("seed").await.unwrap(), ["match"]);
    assert_eq!(api.moods().await.unwrap(), ["calm"]);
    assert_eq!(api.mood("calm").await.unwrap(), ["match"]);
    assert_eq!(
        api.path("seed", "end", 5).await.unwrap(),
        ["seed", "match", "end"]
    );
    assert!(api.path("seed", "seed", 5).await.is_err());
    assert!(api.mood("unavailable").await.is_err());

    let s = server(|r| {
        (
            200,
            if r.path == "auth" {
                json!({})
            } else {
                json!([{"title":"not an ID"}])
            },
        )
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    assert!(api
        .similar("seed")
        .await
        .unwrap_err()
        .to_string()
        .contains("track ID"));
    assert!(api.path("seed", "end", 5).await.is_err());
}

#[tokio::test]
async fn sync_scopes_ids_omits_embeddings_and_fetches_only_changed_analysis() {
    let requested = Arc::new(Mutex::new(Vec::new()));
    let seen = requested.clone();
    let s = server(move |r| {
        if r.path == "auth" { assert_eq!(r.body["user"], "listener"); return (200,json!({"status":"ok"})); }
        assert_eq!(r.query["server"], "My Navidrome");
        match r.path.as_str() {
            "api/servers" => (200,json!({"servers":[{"name":"My Navidrome","server_type":"navidrome"}]})),
            "api/sync" if r.query.get("fields").map(String::as_str) == Some("index") =>
                (200,page(json!([{"id":"keep","fp":"a"},{"id":"update","fp":"b"},{"id":"outside","fp":"c"}]))),
            "api/sync" if r.query.contains_key("ids") => {
                assert_eq!(r.query["include_embeddings"], "false");
                seen.lock().unwrap().push(r.query["ids"].clone());
                (200,page(json!([analysis("update","b")])))
            }
            "api/sync" => (200,page(json!([analysis("keep","a")]))),
            _ => panic!("unexpected endpoint"),
        }
    }).await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    api.verify().await.unwrap();
    let old = Snapshot {
        tracks: BTreeMap::from([
            ("keep".into(), analysis("keep", "a")),
            ("update".into(), analysis("update", "old")),
            ("deleted".into(), analysis("deleted", "z")),
        ]),
        ..Default::default()
    };
    let allowed = HashSet::from(["keep".into(), "update".into(), "deleted".into()]);
    let new = api.refresh(&old, &allowed).await.unwrap();
    assert_eq!(new.tracks.len(), 2);
    assert_eq!(*requested.lock().unwrap(), ["update"]);
    assert!(old.tracks.contains_key("deleted"));
    assert!(!new.tracks.contains_key("outside"));
    let dir = std::env::temp_dir().join(format!("textamp-am-test-{}", uuid::Uuid::new_v4()));
    let ticket = textamp::library::cache::Store::at(
        dir.clone(),
        ("nav-account", "folder", connection(&s.url)),
    )
    .unwrap()
    .ticket("analysis");
    ticket.write(&new).unwrap();
    let hit = ticket
        .read::<Snapshot>(std::time::Duration::from_secs(3600))
        .unwrap()
        .unwrap();
    assert_eq!(hit.value.tracks["update"].fp, "b");
    assert_eq!(hit.value.groups(Feature::Labels).len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sync_batches_fit_the_server_request_line_limit() {
    let ids: Vec<_> = (0..205).map(|i| format!("{i:032x}")).collect();
    let catalog = ids.clone();
    let batches = Arc::new(Mutex::new(Vec::new()));
    let seen = batches.clone();
    let s = server(move |r| {
        if r.path == "auth" {
            return (200, json!({"status":"ok"}));
        }
        if r.query.contains_key("fields") {
            return (
                200,
                page(json!(catalog
                    .iter()
                    .map(|id| json!({"id":id,"fp":"a"}))
                    .collect::<Vec<_>>())),
            );
        }
        let ids: Vec<_> = r.query["ids"].split(',').collect();
        let mut url = reqwest::Url::parse("http://local/prefix/api/sync").unwrap();
        url.query_pairs_mut().extend_pairs(&r.query);
        assert!(url.as_str().len() < 4094);
        seen.lock().unwrap().push(ids.len());
        (
            200,
            page(json!(ids
                .iter()
                .map(|id| analysis(id, "a"))
                .collect::<Vec<_>>())),
        )
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    let result = api
        .refresh(
            &Snapshot {
                tracks: [(ids[0].clone(), analysis(&ids[0], "old"))]
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            &ids.into_iter().collect(),
        )
        .await
        .unwrap();
    assert_eq!(result.tracks.len(), 205);
    assert_eq!(*batches.lock().unwrap(), [100, 100, 5]);
}

#[tokio::test]
async fn sync_does_not_attach_an_alias_response_to_the_requested_track() {
    let s = server(|r| {
        if r.path == "auth" {
            return (200, json!({"status":"ok"}));
        }
        if r.query.contains_key("fields") {
            return (
                200,
                page(json!([{"id":"one","fp":"a"},{"id":"two","fp":"b"}])),
            );
        }
        (
            200,
            page(json!([
                analysis("one", "a"),
                analysis("different-copy", "b")
            ])),
        )
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    let allowed = HashSet::from(["one".into(), "two".into(), "different-copy".into()]);
    let previous = Snapshot {
        tracks: [("deleted".into(), analysis("deleted", "old"))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let result = api.refresh(&previous, &allowed).await.unwrap();
    assert!(result.partial);
    assert_eq!(result.unresolved, 1);
    assert_eq!(result.tracks.len(), 1);
    assert!(result.tracks.contains_key("one"));
    let old = Snapshot {
        tracks: BTreeMap::from([("two".into(), analysis("two", "old"))]),
        ..Default::default()
    };
    let result = api.refresh(&old, &allowed).await.unwrap();
    assert_eq!(result.tracks["two"].fp, "old");
    assert!(!result.tracks.contains_key("different-copy"));
}

#[tokio::test]
async fn incomplete_or_repeated_sync_is_an_error_not_an_empty_success() {
    for repeat in [false, true] {
        let s = server(move |r| {
            if r.path == "auth" {
                return (200, json!({"status":"ok"}));
            }
            if r.query.contains_key("ids") {
                return (200, page(json!([])));
            }
            let mut p = page(json!([{"id":"one","fp":"a"}]));
            if repeat {
                p["has_more"] = json!(true);
                p["next_page"] = json!(r.query["page"].parse::<usize>().unwrap() + 1);
            }
            (200, p)
        })
        .await;
        let api = Client::connect(&connection(&s.url), "fixture-password")
            .await
            .unwrap();
        let result = api
            .refresh(
                &Snapshot {
                    tracks: [("one".into(), analysis("one", "old"))]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                &HashSet::from(["one".into()]),
            )
            .await;
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn cold_sync_reads_full_metadata_pages_once_and_reports_progress() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let s = server(move |r| {
        if r.path == "auth" {
            return (200, json!({"status":"ok"}));
        }
        assert_eq!(r.path, "api/sync");
        assert_eq!(r.query["include_embeddings"], "false");
        assert_eq!(r.query["limit"], "1000");
        assert!(!r.query.contains_key("fields"));
        assert!(!r.query.contains_key("ids"));
        let number = r.query["page"].parse::<usize>().unwrap();
        seen.lock().unwrap().push(number);
        let mut p = match number {
            1 => page(json!([analysis("one", "a"), analysis("outside", "b")])),
            2 => page(json!([analysis("two", "c")])),
            _ => panic!("unnecessary request"),
        };
        p["total_tracks"] = json!(3);
        p["has_more"] = json!(number == 1);
        p["next_page"] = if number == 1 { json!(2) } else { json!(null) };
        (200, p)
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    let mut progress = vec![];
    let result = api
        .refresh_with_progress(
            &Snapshot::default(),
            &HashSet::from(["one".into(), "two".into()]),
            |p| progress.push(p.to_string()),
        )
        .await
        .unwrap();
    assert_eq!(*requests.lock().unwrap(), [1, 2]);
    assert_eq!(
        progress,
        [
            "AudioMuse index: 2/3",
            "AudioMuse: 1 tracks · loading…",
            "AudioMuse index: 3/3"
        ]
    );
    assert_eq!(result.tracks.len(), 2);
    assert!(!result.tracks.contains_key("outside"));
    assert!(!result.partial);
    for feature in [
        Feature::Labels,
        Feature::Tempo,
        Feature::Energy,
        Feature::Key,
    ] {
        assert!(!result.groups(feature).is_empty());
    }
}

#[tokio::test]
async fn cold_sync_does_not_return_partial_success_after_a_failed_or_repeated_page() {
    for repeat in [false, true] {
        let s = server(move |r| {
            if r.path == "auth" {
                return (200, json!({"status":"ok"}));
            }
            let number = r.query["page"].parse::<usize>().unwrap();
            if number > 1 && !repeat {
                return (503, json!({"error":"fixture outage"}));
            }
            let mut p = page(json!([analysis("one", "a")]));
            p["has_more"] = json!(true);
            p["next_page"] = json!(number + 1);
            (200, p)
        })
        .await;
        let api = Client::connect(&connection(&s.url), "fixture-password")
            .await
            .unwrap();
        assert!(api
            .refresh(&Snapshot::default(), &HashSet::from(["one".into()]))
            .await
            .is_err());
    }
}

#[tokio::test]
async fn search_checks_readiness_and_preserves_rank_without_playlist_writes() {
    for enabled in [false, true] {
        let s = server(move |r| match r.path.as_str() {
            "auth" => (200, json!({"status":"ok"})),
            "api/clap/stats" => (200, json!({"clap_enabled":enabled,"song_count":3})),
            "api/lyrics/stats" => (200, json!({"lyrics_enabled":enabled,"song_count":3})),
            "api/clap/search" | "api/lyrics/search/text" => {
                assert!(enabled);
                assert_eq!(r.body["server"], "My Navidrome");
                (200, json!({"results":[{"item_id":"z"},{"item_id":"a"}]}))
            }
            _ => panic!("unexpected write or endpoint"),
        })
        .await;
        let api = Client::connect(&connection(&s.url), "fixture-password")
            .await
            .unwrap();
        for feature in [Feature::Describe, Feature::Lyrics] {
            let result = api.search(feature, "gentle piano").await;
            if enabled {
                assert_eq!(result.unwrap(), ["z", "a"]);
            } else {
                assert!(result.unwrap_err().to_string().contains("disabled"));
            }
        }
    }
}

#[tokio::test]
async fn active_scan_page_overlap_is_partial_and_does_not_delete_cached_analysis() {
    let s=server(|r| {
        if r.path=="auth" { return (200,json!({})); }
        if let Some(ids)=r.query.get("ids") {
            return (200,page(json!(ids.split(',').map(|id| analysis(id,"a")).collect::<Vec<_>>())));
        }
        if r.query["page"]=="1" {
            (200,json!({"tracks":[{"id":"one","fp":"a"},{"id":"two","fp":"a"}],"provider_type":"navidrome","has_more":true,"next_page":2,"total_tracks":3}))
        } else {
            (200,json!({"tracks":[{"id":"two","fp":"a"},{"id":"three","fp":"a"}],"provider_type":"navidrome","has_more":false,"next_page":null,"total_tracks":4}))
        }
    }).await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    let old = Snapshot {
        tracks: BTreeMap::from([("not-seen".into(), analysis("not-seen", "old"))]),
        ..Default::default()
    };
    let allowed = ["one", "two", "three", "not-seen"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let new = api.refresh(&old, &allowed).await.unwrap();
    assert!(new.partial);
    assert_eq!(new.tracks.len(), 4);
    assert_eq!(new.tracks["not-seen"].fp, "old");
}

#[tokio::test]
async fn auth_and_wrong_track_boundaries_fail_without_exposing_secrets() {
    let s = server(|r| {
        if r.path == "auth" {
            (401, json!({"error":"fixture-password"}))
        } else {
            panic!("must not proceed")
        }
    })
    .await;
    let error = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("fixture-password"));
    let s = server(|r| {
        if r.path == "auth" {
            (200, json!({}))
        } else {
            (200, page(json!([analysis("wrong", "a")])))
        }
    })
    .await;
    let api = Client::connect(&connection(&s.url), "fixture-password")
        .await
        .unwrap();
    assert!(api.analysis("right").await.is_err());
    assert!(connection("https://user:secret@example.test")
        .validate()
        .is_err());
}
