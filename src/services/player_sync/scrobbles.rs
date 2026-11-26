//! Import completed, timestamped player listens against the library's managed track IDs.
use super::*;
const LOG: &str = ".scrobbler.log";
pub(super) const RECEIPTS: &str = ".textamp-listens.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Listen {
    artist: String,
    album: String,
    title: String,
    number: Option<u32>,
    seconds: u64,
    start: i64,
}
impl Listen {
    fn parse(line: &str) -> Option<Self> {
        let mut f = line.split('\t');
        let (artist, album, title) = (f.next()?, f.next()?, f.next()?);
        let number = f.next()?.parse().ok().filter(|n| *n > 0);
        let seconds = f.next()?.parse().ok()?;
        if f.next()? != "L" || artist.trim().is_empty() || title.trim().is_empty() {
            return None;
        }
        let start: i64 = f.next()?.parse().ok()?;
        if start <= 0 || time::OffsetDateTime::from_unix_timestamp(start).is_err() {
            return None;
        }
        Some(Self {
            artist: artist.into(),
            album: album.into(),
            title: title.into(),
            number,
            seconds,
            start,
        })
    }
    fn matches(&self, track: &Tags) -> bool {
        fn equal(a: &str, b: &str) -> bool {
            a.trim().to_lowercase() == b.trim().to_lowercase()
        }
        equal(&self.artist, &track.artist)
            && equal(&self.title, &track.title)
            && (self.album.is_empty() || equal(&self.album, &track.album))
            && self.number.zip(track.number).is_none_or(|(a, b)| a == b)
            && track
                .seconds
                .is_none_or(|n| self.seconds == 0 || n.abs_diff(self.seconds) <= 2)
    }
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Tags {
    artist: String,
    album: String,
    title: String,
    number: Option<u32>,
    seconds: Option<u64>,
}
impl From<&Track> for Tags {
    fn from(track: &Track) -> Self {
        Self {
            artist: track.track_artist().into(),
            album: track.album_name().into(),
            title: track.title.clone(),
            number: track.index,
            seconds: track.duration.map(|ms| ms / 1000),
        }
    }
}
pub(super) async fn file_tags(source: &str, name: &str, file: &Path) -> Result<Option<Tags>> {
    let mut track = Track::from_folder(source, name);
    let file = file.to_owned();
    crate::app::tasks::spawn_blocking(move || {
        track
            .read_file_metadata(&file)
            .ok()
            .map(|_| Tags::from(&track))
    })
    .await
    .map_err(Into::into)
}
fn possible_title(path: &str, title: &str) -> bool {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| {
            stem.to_lowercase()
                .contains(&plain_name(title).to_lowercase())
        })
}
async fn other_indexes(
    target: &volumes::Target,
    active: &Path,
    uuid: &str,
) -> Result<Vec<Manifest>> {
    let mut dirs = tokio::fs::read_dir(&target.path).await?;
    let mut indexes = vec![];
    while let Some(dir) = dirs.next_entry().await? {
        if dir.path() == active
            || !dir.file_name().to_string_lossy().starts_with("Textamp ")
            || !dir.file_type().await?.is_dir()
        {
            continue;
        }
        let path = safe_path(&dir.path(), INDEX).await?;
        if tokio::fs::try_exists(&path).await? {
            let index: Manifest =
                serde_json::from_slice(&read_limited(&path, 32 * 1024 * 1024).await?)?;
            ensure!(
                index.volume_uuid == uuid && index.version == 2,
                "Cannot identify another managed library on the player"
            );
            indexes.push(index);
        }
    }
    Ok(indexes)
}
#[derive(Serialize, Deserialize)]
enum Submission {
    Local,
    Pending,
    Submitted,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    source: String,
    track: String,
    listen: Listen,
    submission: Submission,
}

async fn log(target: &volumes::Target) -> Result<Vec<Listen>> {
    let path = safe_path(&target.path, LOG).await?;
    if !tokio::fs::try_exists(&path).await? {
        return Ok(vec![]);
    }
    let bytes = read_limited(&path, 32 * 1024 * 1024).await?;
    let mut lines = std::str::from_utf8(&bytes)?.lines();
    ensure!(
        lines.next() == Some("#AUDIOSCROBBLER/1.1"),
        "Unrecognized player listening log"
    );
    Ok(lines.filter_map(Listen::parse).collect())
}
pub(super) async fn import(
    target: &volumes::Target,
    root: &Path,
    manifest: &mut Manifest,

    client: Option<&Client>,
    resolve: impl Fn(&str) -> crate::library::models::Track,
) -> Result<(usize, usize)> {
    let listens = log(target).await?;
    if listens.is_empty() {
        return Ok((0, 0));
    }
    let others = other_indexes(target, root, &manifest.volume_uuid).await?;
    let path = safe_path(&target.path, RECEIPTS).await?;
    let mut receipts: BTreeMap<String, Receipt> = if tokio::fs::try_exists(&path).await? {
        serde_json::from_slice(&read_limited(&path, 32 * 1024 * 1024).await?)?
    } else {
        BTreeMap::new()
    };
    let mut imported = 0;
    let mut uncertain = 0;
    for listen in listens {
        let key = format!("{:x}", md5::compute(serde_json::to_vec(&listen)?));
        if let Some(old) = receipts.get(&key) {
            if matches!(old.submission, Submission::Pending) {
                uncertain += 1;
            }
            continue;
        }
        if others.iter().any(|index| {
            index.files.iter().any(|(id, entry)| {
                index.listening_tags.get(id).map_or_else(
                    || possible_title(&entry.path, &listen.title),
                    |tags| listen.matches(tags),
                )
            })
        }) {
            continue;
        }
        let mut matches = BTreeSet::new();
        for (id, entry) in &manifest.files {
            if !manifest.listening_tags.contains_key(id)
                && possible_title(&entry.path, &listen.title)
            {
                let file = safe_path(root, &entry.path).await?;
                if tokio::fs::try_exists(&file).await? {
                    target.hydrate(&file).await?;
                    if let Some(tags) = file_tags(&manifest.source_id, &entry.path, &file).await? {
                        manifest.listening_tags.insert(id.clone(), tags);
                    }
                }
            }
            if manifest
                .listening_tags
                .get(id)
                .is_some_and(|tags| listen.matches(tags))
            {
                matches.insert(id.clone());
            }
        }
        if matches.len() != 1 {
            continue;
        }
        let id = matches.pop_first().unwrap();
        let start = listen.start;
        let play = crate::app::listening_history::Play {
            id: format!("player-{}-{key}", manifest.volume_uuid),
            title: listen.title.clone(),
            artist: listen.artist.clone(),
            album: listen.album.clone(),
            time: start,
            track: Some({
                let mut track = resolve(&id);
                track.title.clone_from(&listen.title);
                track.original_title = Some(listen.artist.clone());
                track.parent_title = Some(listen.album.clone());
                track
                    .duration
                    .get_or_insert(listen.seconds.saturating_mul(1000));
                track
            }),
        };
        crate::app::tasks::spawn_blocking(move || crate::app::listening_history::save(Some(play)))
            .await??;
        receipts.insert(
            key.clone(),
            Receipt {
                source: manifest.source_id.clone(),
                track: id.clone(),
                listen,
                submission: if client.is_some() {
                    Submission::Pending
                } else {
                    Submission::Local
                },
            },
        );
        atomic_write(target, &path, &serde_json::to_vec(&receipts)?).await?;
        if let Some(client) = client {
            client.call("scrobble",&[("id",id),("submission","true".into()),("time",(start*1000).to_string())]).await.context("Player listen submission was not confirmed. Its receipt prevents duplicate submissions")?;
            receipts.get_mut(&key).unwrap().submission = Submission::Submitted;
            atomic_write(target, &path, &serde_json::to_vec(&receipts)?).await?;
        }
        imported += 1;
    }
    Ok((imported, uncertain))
}
