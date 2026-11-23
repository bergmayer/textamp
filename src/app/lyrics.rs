//! Structured lyrics and playback timing shared by every frontend.
use super::{Action, AppState, Event, action::PlaybackAction};
use crate::library::models::Track;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Document {
    pub lang: String,
    pub kind: String,
    pub synced: bool,
    pub offset: i64,
    pub line: Vec<Line>,
    pub cue_line: Vec<CueLine>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Line {
    pub value: String,
    pub start: Option<u64>,
    pub end: Option<u64>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct CueLine {
    pub index: usize,
    pub value: String,
    pub cue: Vec<Cue>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cue {
    pub start: u64,
    pub end: u64,
    pub byte_start: Option<usize>,
    pub byte_end: Option<usize>,
}

#[derive(Clone, Debug)]
pub enum Content {
    Loading,
    Empty,
    Ready(Vec<Document>),
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct Panel {
    pub track: Track,
    pub generation: u64,
    pub request: u64,
    pub playback_bound: bool,
    pub content: Content,
    pub language: usize,
    pub follow: bool,
    pub scroll: u16,
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub track: String,
    pub title: String,
    pub message: Option<String>,
    pub loading: bool,
    pub follow: bool,
    pub synced: bool,
    pub language: usize,
    pub languages: Vec<String>,
    pub active: Option<usize>,
    pub lines: Vec<DisplayLine>,
}
#[derive(Clone, Debug, Serialize)]
pub struct DisplayLine {
    pub value: String,
    pub active: bool,
    pub spans: Vec<Span>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub text: String,
    pub active: bool,
}

impl Document {
    pub fn active_line(&self, position: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let position = i128::from(position) + i128::from(self.offset);
        self.line
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.start
                    .is_some_and(|start| i128::from(start) <= position)
            })
            .max_by_key(|(_, line)| line.start)
            .filter(|(_, line)| line.end.is_none_or(|end| position < i128::from(end)))
            .map(|(index, _)| index)
    }
    fn cue_range(&self, index: usize, position: u64) -> Option<std::ops::Range<usize>> {
        let line = &self.line[index];
        let position = i128::from(position) + i128::from(self.offset);
        let cue = self
            .cue_line
            .iter()
            .filter(|cue| cue.index == index && cue.value == line.value)
            .flat_map(|cue| &cue.cue)
            .find(|cue| i128::from(cue.start) <= position && position < i128::from(cue.end));
        cue.and_then(|cue| Some(cue.byte_start?..cue.byte_end?.checked_add(1)?))
            .filter(|range| line.value.get(range.clone()).is_some())
    }
    fn highlight(&self, position: u64) -> Option<(usize, Option<std::ops::Range<usize>>)> {
        let index = self.active_line(position)?;
        Some((index, self.cue_range(index, position)))
    }
    fn spans(&self, index: usize, position: u64) -> Vec<Span> {
        let line = &self.line[index];
        let range = self.cue_range(index, position);
        let clean = |text: &str| crate::util::sanitize_display_text(text).into_owned();
        if let Some(range) = range {
            vec![
                Span {
                    text: clean(&line.value[..range.start]),
                    active: false,
                },
                Span {
                    text: clean(&line.value[range.clone()]),
                    active: true,
                },
                Span {
                    text: clean(&line.value[range.end..]),
                    active: false,
                },
            ]
        } else {
            vec![Span {
                text: clean(&line.value),
                active: false,
            }]
        }
    }
}
impl Panel {
    pub fn search_query(&self) -> String {
        format!("{} {} lyrics", self.track.track_artist(), self.track.title)
    }
    pub fn documents(&self) -> &[Document] {
        match &self.content {
            Content::Ready(documents) => documents,
            _ => &[],
        }
    }
    pub fn view(&self, position: Option<u64>) -> View {
        let document = self.documents().get(self.language);
        let active = position
            .filter(|_| self.follow)
            .and_then(|position| document?.active_line(position));
        View {
            track: self.track.rating_key.clone(),
            title: format!("Lyrics · {}", self.track.title),
            loading: matches!(self.content, Content::Loading),
            follow: self.follow && document.is_some_and(|document| document.synced),
            synced: document.is_some_and(|document| document.synced),
            language: self.language,
            message: match &self.content {
                Content::Loading => Some("Loading lyrics…".into()),
                Content::Empty => Some("No lyrics found".into()),
                Content::Failed(error) => Some(error.clone()),
                Content::Ready(_) => None,
            },
            languages: self
                .documents()
                .iter()
                .enumerate()
                .map(|(index, document)| {
                    let lang = if document.lang.is_empty() {
                        "Unspecified"
                    } else {
                        &document.lang
                    };
                    if document.kind.is_empty() || document.kind == "main" {
                        format!("{lang} · {}", index + 1)
                    } else {
                        format!("{lang} · {}", document.kind)
                    }
                })
                .collect(),
            active,
            lines: document
                .map(|document| {
                    document
                        .line
                        .iter()
                        .enumerate()
                        .map(|(index, line)| {
                            let spans = if active == Some(index) {
                                document.spans(index, position.unwrap_or(0))
                            } else {
                                vec![Span {
                                    text: crate::util::sanitize_display_text(&line.value)
                                        .into_owned(),
                                    active: false,
                                }]
                            };
                            DisplayLine {
                                value: spans.iter().map(|span| span.text.as_str()).collect(),
                                active: active == Some(index),
                                spans,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

pub fn view(state: &AppState) -> Option<View> {
    let panel = state.popups.lyrics()?;
    let position = state
        .current_track()
        .filter(|track| track.rating_key == panel.track.rating_key)
        .map(|_| state.playback.position_ms);
    Some(panel.view(position))
}

/// Emit a clock-only update when playback crosses a visible line or word boundary.
pub fn playback_update(state: &AppState, previous: u64) -> Option<View> {
    let panel = state.popups.lyrics()?;
    if !panel.follow || state.current_track()?.rating_key != panel.track.rating_key {
        return None;
    }
    let document = panel.documents().get(panel.language)?;
    let position = state.playback.position_ms;
    (document.highlight(previous) != document.highlight(position))
        .then(|| panel.view(Some(position)))
}

pub fn open(
    state: &mut AppState,
    events: &mpsc::Sender<Event>,
    track: Track,
    playback_bound: bool,
) {
    let follow = state.popups.lyrics().is_none_or(|panel| panel.follow);
    state.sources.nav_tasks.remove("lyrics");
    let request = state.sources.nav_request_id.wrapping_add(1);
    let generation = state.library_generation;
    let session = state.sources.active.navidrome().cloned();
    let offline = state.downloads.is_offline();
    state.popups.set_lyrics(Some(Panel {
        track: track.clone(),
        generation,
        request,
        playback_bound,
        content: Content::Loading,
        language: 0,
        follow,
        scroll: 0,
    }));
    if let Some(session) = session {
        super::sources::navidrome::effects::spawn(state, events, "lyrics", async move {
            let result = load(&session, &track, offline)
                .await
                .map_err(|error| format!("Could not load lyrics: {error}"));
            Ok(vec![
                PlaybackAction::LyricsLoaded {
                    generation,
                    request,
                    result,
                }
                .into(),
            ])
        });
    } else {
        state.sources.nav_request_id = request;
        let documents = track
            .lyrics
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![Document {
                    line: text
                        .lines()
                        .map(|value| Line {
                            value: value.into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }]
            })
            .unwrap_or_default();
        complete(state, generation, request, Ok(documents));
    }
}

pub fn complete(
    state: &mut AppState,
    generation: u64,
    request: u64,
    result: Result<Vec<Document>, String>,
) {
    if generation != state.library_generation {
        return;
    }
    if state.popups.lyrics().is_some_and(|panel| {
        panel.playback_bound
            && state
                .current_track()
                .is_none_or(|track| track.rating_key != panel.track.rating_key)
    }) {
        return;
    }
    let Some(panel) = state
        .popups
        .lyrics_mut()
        .filter(|panel| panel.request == request && panel.generation == generation)
    else {
        return;
    };
    panel.content = match result {
        Ok(result) if result.is_empty() => Content::Empty,
        Ok(result) => Content::Ready(result),
        Err(error) => Content::Failed(error),
    };
    panel.language = panel
        .language
        .min(panel.documents().len().saturating_sub(1));
}

/// Track changes replace the owned request; closed panels cancel their request lease.
pub fn reconcile(state: &mut AppState, events: &mpsc::Sender<Event>) {
    let Some(panel) = state.popups.lyrics() else {
        state.sources.nav_tasks.remove("lyrics");
        return;
    };
    if panel.generation != state.library_generation
        || (panel.playback_bound && state.current_track().is_none())
    {
        state.sources.nav_tasks.remove("lyrics");
        state.popups.set_lyrics(None);
        return;
    }
    if panel.playback_bound
        && let Some(track) = state
            .current_track()
            .filter(|track| track.rating_key != panel.track.rating_key)
            .cloned()
    {
        open(state, events, track, true);
    }
}

async fn load(
    session: &super::sources::navidrome::Session,
    track: &Track,
    offline: bool,
) -> anyhow::Result<Vec<Document>> {
    if offline {
        return track
            .lyrics
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|text| {
                vec![Document {
                    line: text
                        .lines()
                        .map(|value| Line {
                            value: value.into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }]
            })
            .ok_or_else(|| anyhow::anyhow!("Lyrics are unavailable offline"));
    }
    let response = session
        .client
        .call(
            "getLyricsBySongId",
            &[
                ("id", session.id(&track.rating_key)?),
                ("enhanced", "true".into()),
            ],
        )
        .await?;
    let mut documents: Vec<Document> = if response
        .get("lyricsList")
        .is_none_or(serde_json::Value::is_null)
    {
        vec![]
    } else {
        crate::navidrome::array(&response["lyricsList"], "structuredLyrics")?
    };
    documents.retain(|document| {
        document
            .line
            .iter()
            .any(|line| !line.value.trim().is_empty())
    });
    Ok(documents)
}

pub fn web_search(query: &str) -> Action {
    super::action::SystemAction::OpenBrowser(format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    ))
    .into()
}

#[derive(Clone, Copy, Debug)]
pub enum Control {
    Search,
    ToggleSync,
    NextLanguage,
    Retry,
}
impl Control {
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Search Lyrics on the Web",
            Self::ToggleSync => "Follow Lyrics During Playback",
            Self::NextLanguage => "Next Lyrics Language",
            Self::Retry => "Reload Lyrics",
        }
    }
    pub fn actions(self, state: &AppState) -> Vec<Action> {
        match self {
            Self::Search => state
                .popups
                .lyrics()
                .map(|panel| vec![web_search(&panel.search_query())])
                .unwrap_or_default(),
            Self::ToggleSync => vec![PlaybackAction::ToggleLyricsSync.into()],
            Self::NextLanguage => state
                .popups
                .lyrics()
                .filter(|panel| panel.documents().len() > 1)
                .map(|panel| {
                    vec![
                        PlaybackAction::SelectLyricsLanguage(
                            (panel.language + 1) % panel.documents().len(),
                        )
                        .into(),
                    ]
                })
                .unwrap_or_default(),
            Self::Retry => vec![PlaybackAction::RetryLyrics.into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel(documents: Vec<Document>) -> Panel {
        Panel {
            track: serde_json::from_value(serde_json::json!({"ratingKey":"song","title":"Song"}))
                .unwrap(),
            generation: 0,
            request: 1,
            playback_bound: true,
            content: Content::Ready(documents),
            language: 0,
            follow: true,
            scroll: 0,
        }
    }
    fn document() -> Document {
        serde_json::from_value(serde_json::json!({"lang":"en","synced":true,"line":[{"value":"First","start":1000},{"value":"한 word","start":2000,"end":4000}],"cueLine":[{"index":1,"value":"한 word","cue":[{"start":2000,"end":2500,"byteStart":0,"byteEnd":2},{"start":2500,"end":3500,"byteStart":4,"byteEnd":7}]}]})).unwrap()
    }
    #[tokio::test]
    async fn replacing_and_closing_a_panel_cancel_owned_requests() {
        let mut state = AppState::new();
        let (events, _) = mpsc::channel(4);
        let mut p = panel(vec![document()]);
        p.generation = state.library_generation;
        let mut next = p.track.clone();
        next.rating_key = "next".into();
        state.queue.tracks = vec![next];
        state.queue.index = Some(0);
        state.popups.set_lyrics(Some(p));
        let task = tokio::spawn(std::future::pending::<()>());
        state
            .sources
            .nav_tasks
            .insert("lyrics", (1, super::super::tasks::TaskLease::new(&task)));
        reconcile(&mut state, &events);
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(state.popups.lyrics().unwrap().track.rating_key, "next");
        let task = tokio::spawn(std::future::pending::<()>());
        state
            .sources
            .nav_tasks
            .insert("lyrics", (2, super::super::tasks::TaskLease::new(&task)));
        state.popups.set_lyrics(None);
        reconcile(&mut state, &events);
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!state.sources.nav_tasks.contains_key("lyrics"));
    }

    #[test]
    fn seeking_recomputes_lines_and_utf8_word_boundaries() {
        let p = panel(vec![document()]);
        assert_eq!(p.view(Some(500)).active, None);
        assert_eq!(p.view(Some(2700)).active, Some(1));
        assert_eq!(
            p.view(Some(2700)).lines[1]
                .spans
                .iter()
                .find(|s| s.active)
                .unwrap()
                .text,
            "word"
        );
        assert_eq!(
            p.view(Some(2100)).lines[1]
                .spans
                .iter()
                .find(|s| s.active)
                .unwrap()
                .text,
            "한"
        );
        assert_eq!(p.view(Some(1500)).active, Some(0));
        assert_eq!(p.view(Some(4100)).active, None);
        let mut invalid = document();
        invalid.cue_line[0].cue[0].byte_start = Some(1);
        let view = panel(vec![invalid]).view(Some(2100));
        assert_eq!(view.lines[1].value, "한 word");
        assert!(!view.lines[1].spans.iter().any(|s| s.active));
    }
    #[test]
    fn clock_updates_follow_highlight_changes_within_a_line() {
        let mut state = AppState::new();
        let panel = panel(vec![document()]);
        state.queue.tracks = vec![panel.track.clone()];
        state.queue.index = Some(0);
        state.popups.set_lyrics(Some(panel));
        state.playback.position_ms = 2600;
        assert!(playback_update(&state, 2500).is_none());
        let update = playback_update(&state, 2400).unwrap();
        assert_eq!(update.active, Some(1));
        assert!(
            update.lines[1]
                .spans
                .iter()
                .any(|span| span.active && span.text == "word")
        );
        state.playback.position_ms = 4000;
        assert_eq!(playback_update(&state, 2600).unwrap().active, None);
        state.popups.lyrics_mut().unwrap().follow = false;
        assert!(playback_update(&state, 2600).is_none());
    }

    #[test]
    fn follow_toggle_and_language_do_not_discard_text() {
        let mut plain = document();
        plain.synced = false;
        plain.lang = "fr".into();
        let mut p = panel(vec![document(), plain]);
        p.follow = false;
        let view = p.view(Some(2100));
        assert_eq!(view.active, None);
        assert_eq!(view.lines.len(), 2);
        p.follow = true;
        p.language = 1;
        let view = p.view(Some(2100));
        assert_eq!(view.active, None);
        assert!(!view.synced);
        assert_eq!(view.languages.len(), 2);
        p.language = 0;
        assert_eq!(p.view(Some(2100)).active, Some(1));
    }
    #[test]
    fn completions_distinguish_empty_failure_and_reject_stale_owners() {
        let mut state = AppState::new();
        let mut p = panel(vec![]);
        p.generation = state.library_generation;
        state.queue.tracks = vec![p.track.clone()];
        state.queue.index = Some(0);
        let generation = state.library_generation;
        state.popups.set_lyrics(Some(p));
        complete(&mut state, generation, 1, Ok(vec![]));
        assert_eq!(
            view(&state).unwrap().message.as_deref(),
            Some("No lyrics found")
        );
        complete(&mut state, generation, 1, Err("Network unavailable".into()));
        assert_eq!(
            view(&state).unwrap().message.as_deref(),
            Some("Network unavailable")
        );
        complete(&mut state, generation, 2, Ok(vec![document()]));
        assert!(matches!(
            state.popups.lyrics().unwrap().content,
            Content::Failed(_)
        ));
        state.queue.tracks[0].rating_key = "other".into();
        complete(&mut state, generation, 1, Ok(vec![document()]));
        assert!(matches!(
            state.popups.lyrics().unwrap().content,
            Content::Failed(_)
        ));
        state.popups.set_lyrics(None);
        complete(&mut state, generation, 1, Ok(vec![document()]));
        assert!(state.popups.lyrics().is_none());
    }
}
