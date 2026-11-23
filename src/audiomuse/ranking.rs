//! Exact distance rankings from the AudioMuse Distant Rank plugin.
use super::Client;
use anyhow::{Context, Result, ensure};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) const WHOLE_QUEUE_LIMIT: usize = 1000;
pub(crate) const PER_TRACK_LIMIT: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    WholeQueue,
    PerTrack,
}

#[derive(Serialize)]
pub(crate) struct Query<'a> {
    pub mode: Mode,
    pub queue: &'a [String],
    pub exclude: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_folder_id: Option<&'a str>,
    pub k: usize,
}

#[derive(Deserialize)]
struct Response {
    server_id: String,
    scope: Scope,
    offset: usize,
    generation: String,
    #[serde(flatten)]
    rankings: Rankings,
}

#[derive(Deserialize)]
struct Scope {
    server_enforced: bool,
    music_folder_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Candidate {
    pub id: String,
}

#[derive(Deserialize)]
pub(crate) struct Position {
    position: usize,
    id: String,
    pub candidates: Vec<Candidate>,
    pub next_offset: Option<usize>,
    error: Option<PluginError>,
}

impl Position {
    pub fn analysis_unavailable(&self) -> bool {
        self.error.is_some()
    }
}

#[derive(Deserialize)]
struct PluginError {
    code: String,
}

pub(crate) struct Page {
    pub generation: String,
    pub rankings: Rankings,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Rankings {
    WholeQueue {
        candidates: Vec<Candidate>,
        next_offset: Option<usize>,
    },
    PerTrack {
        positions: Vec<Position>,
    },
}

#[derive(Debug)]
pub(crate) struct IndexChanged;

impl std::fmt::Display for IndexChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioMuse refreshed its rankings. Try the queue edit again.")
    }
}
impl std::error::Error for IndexChanged {}

impl Client {
    pub(crate) async fn rank(
        &self,
        query: &Query<'_>,
        offset: usize,
        generation: Option<&str>,
    ) -> Result<Page> {
        self.verify().await?;
        let server = self
            .verified_server
            .get()
            .and_then(Option::as_deref)
            .context("AudioMuse did not report the Navidrome server ID")?;
        #[derive(Serialize)]
        struct Request<'a> {
            server_id: &'a str,
            #[serde(flatten)]
            query: &'a Query<'a>,
            offset: usize,
            #[serde(skip_serializing_if = "Option::is_none")]
            generation: Option<&'a str>,
        }
        let body = serde_json::to_vec(&Request {
            server_id: server,
            query,
            offset,
            generation,
        })?;
        ensure!(
            body.len() <= 262_144,
            "This queue exceeds AudioMuse's ranking request size"
        );
        let url = self.base.join("plugins/distant_rank/v1/rank")?;
        let mut retry_delays = [Duration::from_millis(350), Duration::from_secs(1)].into_iter();
        let (status, bytes) = loop {
            let (status, bytes) = self
                .receive(
                    self.http
                        .post(url.clone())
                        .timeout(Duration::from_secs(120))
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body.clone()),
                )
                .await?;
            if matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
            ) && let Some(delay) = retry_delays.next()
            {
                tokio::time::sleep(delay).await;
                continue;
            }
            break (status, bytes);
        };
        if !status.is_success() {
            return Err(ranking_error(status, &bytes));
        }
        let response: Response =
            serde_json::from_slice(&bytes).context("Invalid AudioMuse ranking response")?;
        ensure!(
            response.server_id == server
                && response.scope.server_enforced
                && response.scope.music_folder_id.as_deref() == query.music_folder_id
                && response.offset == offset
                && !response.generation.is_empty(),
            "AudioMuse returned rankings for a different request"
        );
        if generation.is_some_and(|generation| generation != response.generation) {
            return Err(IndexChanged.into());
        }
        let validate_offset = |next_offset: Option<usize>, count: usize| {
            ensure!(
                count <= query.k
                    && next_offset.is_none_or(|next| next > offset && next <= 1_500_000),
                "AudioMuse returned invalid ranking pagination"
            );
            Ok::<_, anyhow::Error>(())
        };
        match (&response.rankings, query.mode) {
            (
                Rankings::WholeQueue {
                    candidates,
                    next_offset,
                },
                Mode::WholeQueue,
            ) => {
                validate_offset(*next_offset, candidates.len())?;
            }
            (Rankings::PerTrack { positions }, Mode::PerTrack) => {
                ensure!(
                    positions.len() == query.queue.len(),
                    "AudioMuse omitted queue positions"
                );
                for (i, (position, id)) in positions.iter().zip(query.queue).enumerate() {
                    ensure!(
                        position.position == i && position.id == *id,
                        "AudioMuse returned a different queue order"
                    );
                    validate_offset(position.next_offset, position.candidates.len())?;
                }
            }
            _ => anyhow::bail!("AudioMuse returned rankings for a different mode"),
        }
        Ok(Page {
            generation: response.generation,
            rankings: response.rankings,
        })
    }
}

fn ranking_error(status: StatusCode, body: &[u8]) -> anyhow::Error {
    #[derive(Deserialize)]
    struct ErrorResponse {
        error: PluginError,
    }
    let error = serde_json::from_slice::<ErrorResponse>(body).ok();
    let code = error.as_ref().map(|error| error.error.code.as_str());
    match (status.as_u16(), code) {
        (401 | 403, _) => anyhow::anyhow!(
            "AudioMuse ranking could not authenticate. Check the AudioMuse connection."
        ),
        (404, Some("UNKNOWN_SERVER")) => anyhow::anyhow!(
            "Distant Rank could not find the configured Navidrome server in AudioMuse."
        ),
        (404 | 405 | 501, _) => {
            anyhow::anyhow!("Enable the Distant Rank plugin on the configured AudioMuse server.")
        }
        (409, _) => IndexChanged.into(),
        (422, Some("SEED_ANALYSIS_UNAVAILABLE")) => {
            anyhow::anyhow!("AudioMuse has no compatible analysis for one or more queued tracks.")
        }
        (429, _) => anyhow::anyhow!("AudioMuse ranking is busy. Try again shortly."),
        (503, _) => anyhow::anyhow!(
            "The AudioMuse ranking index is unavailable. Check the server and try again."
        ),
        _ => anyhow::anyhow!("AudioMuse ranking failed (HTTP {})", status.as_u16()),
    }
}
