//! Object-level access to a USB player through the native session worker.
use super::super::{INDEX, MARKER, Volume};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
};

const ROOT: u32 = u32::MAX;
pub(crate) const PROFILE: &str = ".textamp-player.json";
const LIMIT: u64 = 32 * 1024 * 1024;
#[derive(Clone, Deserialize)]
struct Object {
    id: u32,
    name: String,
    folder: bool,
    size: u64,
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Disconnected(String);

pub(crate) fn reconnectable(error: &anyhow::Error) -> bool {
    error.is::<Disconnected>()
        || error.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            )
        })
}

struct Worker {
    program: PathBuf,
    child: Child,
    output: BufReader<ChildStdout>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Worker {
    fn open(location: u32, program: &Path) -> Result<Self> {
        let mut child = Command::new(program)
            .arg("--mtp-worker")
            .arg(location.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let output = BufReader::new(child.stdout.take().context("Missing MTP response stream")?);
        Ok(Self {
            program: program.into(),
            child,
            output,
        })
    }
    fn call<T: DeserializeOwned>(&mut self, cmd: Value) -> Result<T> {
        let input = self
            .child
            .stdin
            .as_mut()
            .context("MTP player disconnected")?;
        serde_json::to_writer(&mut *input, &cmd)?;
        input.write_all(b"\n")?;
        input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            return Err(Disconnected("MTP connection closed".into()).into());
        }
        match serde_json::from_str(&line) {
            Ok(value) => Ok(value),
            Err(parse_error) => {
                #[derive(Deserialize)]
                struct WorkerError {
                    error: String,
                    #[serde(default)]
                    reconnect: bool,
                }
                if let Ok(reply) = serde_json::from_str::<WorkerError>(&line) {
                    if reply.reconnect {
                        return Err(Disconnected(reply.error).into());
                    }
                    anyhow::bail!(
                        "MTP {}: {}",
                        cmd["op"].as_str().unwrap_or("request"),
                        reply.error
                    );
                }
                Err(parse_error).context("Invalid MTP worker response")
            }
        }
    }
    fn list(&mut self, storage: u32, parent: u32) -> Result<Vec<Object>> {
        let items: Vec<Object> =
            self.call(json!({"op":"list","storage":storage,"parent":parent}))?;
        let mut names = std::collections::HashSet::new();
        for item in &items {
            ensure!(
                Path::new(&item.name)
                    .file_name()
                    .is_some_and(|n| n == item.name.as_str())
                    && names.insert(item.name.to_lowercase()),
                "Ambiguous or invalid MTP object name"
            );
        }
        Ok(items)
    }
    fn directory(&mut self, storage: u32, parent: u32, repair: bool) -> Result<Vec<Object>> {
        let mut entries = self.list(storage, parent)?;
        let journals: Vec<_> = entries
            .iter()
            .filter_map(|item| {
                let token = item.name.strip_prefix(".textamp-")?.strip_suffix(".txn")?;
                if uuid::Uuid::parse_str(token).ok()?.to_string() != token {
                    return None;
                }
                Some((item.clone(), token.to_owned()))
            })
            .collect();
        for (journal, token) in journals {
            ensure!(!journal.folder, "Invalid MTP replacement receipt");
            let old = format!(".textamp-{token}.old");
            let staged = format!(".textamp-{token}.new");
            if !entries.iter().any(|o| o.name == old || o.name == staged) {
                // A receipt alone has no pending rename or backup to restore.
                if repair {
                    self.call::<bool>(json!({"op":"delete","id":journal.id}))?;
                }
                entries.retain(|o| o.id != journal.id);
                continue;
            }
            let file = tempfile::NamedTempFile::new()?;
            self.read(&journal, file.path(), 4096)?;
            let (owner, name): (String, String) =
                serde_json::from_reader(fs::File::open(file.path())?)?;
            ensure!(
                owner == token
                    && Path::new(&name)
                        .file_name()
                        .is_some_and(|n| n == name.as_str()),
                "Invalid MTP replacement receipt"
            );
            let mut restore = entries
                .iter()
                .find(|o| o.name == old)
                .cloned()
                .filter(|_| !entries.iter().any(|o| o.name == name));
            if repair {
                if let Some(obj) = &restore {
                    self.call::<bool>(json!({"op":"rename","id":obj.id,"name":name}))?;
                }
                for o in entries
                    .iter()
                    .filter(|o| o.name == staged || (o.name == old && restore.is_none()))
                {
                    ensure!(!o.folder, "Invalid MTP transfer object");
                    self.call::<bool>(json!({"op":"delete","id":o.id}))?;
                }
                self.call::<bool>(json!({"op":"delete","id":journal.id}))?;
            }
            entries.retain(|o| o.name != staged && o.name != old && o.id != journal.id);
            if let Some(mut obj) = restore.take() {
                obj.name = name;
                entries.push(obj);
            }
        }
        Ok(entries)
    }
    fn read(&mut self, obj: &Object, dest: &Path, limit: u64) -> Result<()> {
        ensure!(
            !obj.folder && obj.size <= limit,
            "Player object exceeds the size limit"
        );
        self.call::<bool>(json!({"op":"read","id":obj.id,"path":dest,"limit":limit}))?;
        Ok(())
    }
}
#[derive(Deserialize)]
struct Storage {
    name: String,
    device: String,
    capacity: u64,
    available: u64,
}
impl Storage {
    fn identity(&self, storage: u32) -> Result<String> {
        ensure!(
            !self.device.is_empty(),
            "MTP player does not provide a stable device identity"
        );
        Ok(
            uuid::Uuid::from_bytes(md5::compute(format!("mtp/{}/{storage}", self.device)).0)
                .to_string(),
        )
    }
}
pub(crate) fn scan(location: u32, name: &str) -> Result<Vec<Volume>> {
    let mut worker = Worker::open(location, &std::env::current_exe()?)?;
    let mut result = vec![];
    for storage in worker.call::<Vec<u32>>(json!({"op":"stores"}))? {
        let items = worker.directory(storage, ROOT, false)?;
        let info: Storage = worker.call(json!({"op":"storage","storage":storage}))?;
        let (uuid, profile_bytes) = if let Some(profile) = items.iter().find(|o| o.name == PROFILE)
        {
            let temporary = tempfile::NamedTempFile::new()?;
            worker.read(profile, temporary.path(), 4 * 1024 * 1024)?;
            let bytes = fs::read(temporary.path())?;
            let data: Value = serde_json::from_slice(&bytes)?;
            let uuid = data["volume_uuid"]
                .as_str()
                .context("Missing player identity")?
                .to_owned();
            uuid::Uuid::parse_str(&uuid)?;
            (uuid, Some(bytes))
        } else {
            (info.identity(storage)?, None)
        };
        let mount = crate::util::paths::get_cache_dir("textamp")
            .context("No cache directory")?
            .join("mtp")
            .join(&uuid);
        fs::create_dir_all(&mount)?;
        if let Some(bytes) = profile_bytes {
            write_cache(&mount.join(PROFILE), &bytes)?;
        } else if mount.join(PROFILE).exists() {
            fs::remove_file(mount.join(PROFILE))?;
        }
        for dir in items
            .iter()
            .filter(|o| o.folder && o.name.starts_with("Textamp "))
        {
            let path = mount.join(&dir.name);
            fs::create_dir_all(&path)?;
            if let Some(index) = worker
                .directory(storage, dir.id, false)?
                .iter()
                .find(|o| o.name == INDEX)
            {
                worker.read(index, &path.join(INDEX), LIMIT)?;
            } else if path.join(INDEX).exists() {
                fs::remove_file(path.join(INDEX))?;
            }
        }
        result.push(Volume {
            uuid: uuid.into(),
            name: format!("{name} · {} (MTP)", info.name),
            mount,
            identifier: format!("mtp:{location}:{storage}"),
            available: info.available,
            capacity: info.capacity,
        });
    }
    Ok(result)
}
fn write_cache(path: &Path, data: &[u8]) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing cache directory")?)?;
    file.write_all(data)?;
    file.persist(path)?;
    Ok(())
}
fn object_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

pub(crate) struct Session {
    worker: Worker,
    storage: u32,
    root: PathBuf,
    objects: HashMap<String, Object>,
    pub(crate) work: tempfile::TempDir,
    cache: PathBuf,
    device_identity: String,
}
impl Session {
    pub(crate) fn open(v: &Volume) -> Result<Self> {
        Self::open_verified(v, None, &std::env::current_exe()?)
    }
    pub(crate) fn open_verified(
        v: &Volume,
        expected_device: Option<&str>,
        program: &Path,
    ) -> Result<Self> {
        let address = v
            .identifier
            .strip_prefix("mtp:")
            .and_then(|v| v.split_once(':'))
            .context("Invalid MTP storage")?;
        let storage = address.1.parse()?;
        let mut worker = Worker::open(address.0.parse()?, program)?;
        let work = tempfile::tempdir()?;
        let root = work.path().canonicalize()?;
        let info: Storage = worker.call(json!({"op":"storage","storage":storage}))?;
        let device_identity = info.identity(storage)?;
        ensure!(
            expected_device.is_none_or(|id| id == device_identity),
            "The selected player changed"
        );
        let entries = worker.directory(storage, ROOT, false)?;
        let objects: HashMap<_, _> = entries
            .into_iter()
            .map(|o| (object_key(&root.join(&o.name)), o))
            .collect();
        if let Some(profile) = objects.get(&object_key(&root.join(PROFILE))) {
            worker.read(profile, &root.join(PROFILE), 4 * 1024 * 1024)?;
            let data: Value = serde_json::from_reader(fs::File::open(root.join(PROFILE))?)?;
            ensure!(
                data["volume_uuid"].as_str() == Some(&v.uuid),
                "The selected player changed"
            );
        } else {
            ensure!(device_identity == v.uuid, "The player identity disappeared");
        }
        let objects = worker
            .directory(storage, ROOT, true)?
            .into_iter()
            .map(|o| (object_key(&root.join(&o.name)), o))
            .collect();
        Ok(Self {
            device_identity,
            worker,
            storage,
            root,
            objects,
            work,
            cache: v.mount.clone(),
        })
    }
    pub(crate) fn reconnect(&mut self, volume: &Volume, source: Option<&str>) -> Result<()> {
        self.worker.child.kill().ok();
        self.worker.child.wait()?;
        let mut fresh =
            Self::open_verified(volume, Some(&self.device_identity), &self.worker.program)?;
        let library = source.map(|id| format!("Textamp {}", super::super::plain_name(id)));
        let mut stack = Vec::new();
        let mut objects = HashMap::new();
        for object in fresh.objects.into_values() {
            let path = self.root.join(&object.name);
            if object.folder
                && object.name.starts_with("Textamp ")
                && library.as_ref().is_none_or(|active| *active == object.name)
            {
                stack.push((path.clone(), object.id));
            }
            objects.insert(object_key(&path), object);
        }
        while let Some((path, parent)) = stack.pop() {
            for object in fresh.worker.directory(self.storage, parent, true)? {
                let child = path.join(&object.name);
                if object.folder {
                    stack.push((child.clone(), object.id));
                }
                objects.insert(object_key(&child), object);
            }
        }
        self.worker = fresh.worker;
        self.objects = objects;
        Ok(())
    }
    pub(crate) fn snapshot(&mut self, source: Option<&str>) -> Result<()> {
        if let Some(receipt) = self.objects.get(&object_key(
            &self.root.join(super::super::scrobbles::RECEIPTS),
        )) {
            self.worker.read(
                receipt,
                &self.root.join(super::super::scrobbles::RECEIPTS),
                LIMIT,
            )?;
        }
        if let Some(source) = source {
            let active = format!("Textamp {}", super::super::plain_name(source));
            for o in self.objects.values() {
                if !o.folder || !o.name.starts_with("Textamp ") || o.name == active {
                    continue;
                }
                if let Some(index) = self
                    .worker
                    .directory(self.storage, o.id, false)?
                    .iter()
                    .find(|o| o.name == INDEX)
                {
                    let dir = self.root.join(&o.name);
                    fs::create_dir_all(&dir)?;
                    self.worker.read(index, &dir.join(INDEX), LIMIT)?;
                }
            }
        }
        let mut stack: Vec<_> = self
            .objects
            .values()
            .filter(|o| {
                o.folder
                    && o.name.starts_with("Textamp ")
                    && source.is_none_or(|id| {
                        o.name == format!("Textamp {}", super::super::plain_name(id))
                    })
            })
            .map(|o| (self.root.join(&o.name), o.id))
            .collect();
        while let Some((dir, parent)) = stack.pop() {
            fs::create_dir_all(&dir)?;
            for obj in self.worker.directory(self.storage, parent, true)? {
                let path = dir.join(&obj.name);
                ensure!(!path.exists(), "Duplicate player path");
                if obj.folder {
                    stack.push((path.clone(), obj.id));
                } else if obj.name == INDEX
                    || obj.name == super::super::scrobbles::RECEIPTS
                    || obj.name == MARKER
                    || obj.name.ends_with(".m3u")
                {
                    self.worker.read(&obj, &path, LIMIT)?;
                } else {
                    fs::File::create_new(&path)?.set_len(obj.size)?;
                }
                self.objects.insert(object_key(&path), obj);
            }
        }
        let stores: Vec<u32> = self.worker.call(json!({"op":"stores"}))?;
        for storage in stores {
            if let Some(log) = self
                .worker
                .list(storage, ROOT)?
                .iter()
                .find(|o| o.name == ".scrobbler.log")
            {
                let path = self.root.join(".scrobbler.log");
                if path.exists() {
                    let part = tempfile::NamedTempFile::new()?;
                    self.worker.read(log, part.path(), LIMIT)?;
                    let bytes = fs::read(part.path())?;
                    if bytes != fs::read(&path)? {
                        let mut f = fs::OpenOptions::new().append(true).open(path)?;
                        f.write_all(b"\n")?;
                        f.write_all(&bytes)?;
                    }
                } else {
                    self.worker.read(log, &path, LIMIT)?;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn hydrate(&mut self, path: &Path) -> Result<()> {
        if let Some(obj) = self.objects.get(&object_key(path)) {
            self.worker.read(obj, path, u32::MAX.into())?;
        }
        Ok(())
    }
    fn parent(&mut self, path: &Path) -> Result<u32> {
        ensure!(path.starts_with(&self.root), "Outside the player");
        if path == self.root {
            return Ok(ROOT);
        }
        if let Some(o) = self.objects.get(&object_key(path)) {
            ensure!(o.folder, "Expected player folder");
            return Ok(o.id);
        }
        let parent = self.parent(path.parent().context("Invalid player path")?)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid player folder")?;
        let id = self.worker.call(
            json!({"op":"create","storage":self.storage,"parent":parent,"name":name,"folder":true}),
        )?;
        self.objects.insert(
            object_key(path),
            Object {
                id,
                name: name.into(),
                folder: true,
                size: 0,
            },
        );
        Ok(id)
    }
    pub(crate) fn put(&mut self, path: &Path, file: &Path) -> Result<()> {
        let parent = self.parent(path.parent().context("Missing player directory")?)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid filename")?;
        let size = fs::metadata(file)?.len();
        let existing = self.objects.get(&object_key(path)).cloned();
        let children = self.worker.list(self.storage, parent)?;
        let folded = name.to_lowercase();
        let actual = children.iter().find(|o| o.name.to_lowercase() == folded);
        ensure!(
            actual.map(|o| (o.id, o.size, o.folder))
                == existing.as_ref().map(|o| (o.id, o.size, false)),
            "Player file changed: {name}"
        );
        let token = uuid::Uuid::new_v4();
        let staged = format!(".textamp-{token}.new");
        let mut receipt = tempfile::NamedTempFile::new()?;
        serde_json::to_writer(&mut receipt, &(token.to_string(), name))?;
        let journal = self
            .worker
            .call::<u32>(json!({
                "op": "create", "name": format!(".textamp-{token}.txn"),
                "storage": self.storage, "parent": parent, "folder": false, "path": receipt.path()
            }))
            .with_context(|| format!("Creating transfer receipt for {name}"))?;
        let id = self
            .worker
            .call::<u32>(json!({
                "op": "create", "name": staged, "storage": self.storage,
                "parent": parent, "folder": false, "path": file
            }))
            .with_context(|| format!("Uploading {name}"))?;
        let uploaded = self.worker.list(self.storage, parent)?;
        if !uploaded
            .iter()
            .any(|o| o.id == id && o.size == size && o.name == staged)
        {
            return Err(Disconnected(format!("Incomplete upload of {name}")).into());
        }
        if let Some(old) = &existing {
            self.worker.call::<bool>(
                json!({"op":"rename","id":old.id,"name":format!(".textamp-{token}.old")}),
            )?;
        }
        self.worker
            .call::<bool>(json!({"op":"rename","id":id,"name":name}))?;
        if let Some(old) = existing {
            self.worker
                .call::<bool>(json!({"op":"delete","id":old.id}))?;
        }
        self.worker
            .call::<bool>(json!({"op":"delete","id":journal}))?;
        if name == INDEX {
            let cache = self.cache.join(path.strip_prefix(&self.root)?);
            fs::create_dir_all(cache.parent().context("Missing manifest directory")?)?;
            write_cache(&cache, &fs::read(file)?)?;
        }
        self.objects.insert(
            object_key(path),
            Object {
                id,
                name: name.into(),
                size,
                folder: false,
            },
        );
        Ok(())
    }
    pub(crate) fn remove(&mut self, path: &Path) -> Result<()> {
        if let Some(obj) = self.objects.get(&object_key(path)) {
            ensure!(!obj.folder, "Expected a player file");
            self.worker
                .call::<bool>(json!({"op":"delete","id":obj.id}))?;
            self.objects.remove(&object_key(path));
        }
        Ok(())
    }
}
pub(crate) fn update_metadata<T: Serialize>(
    volume: &Volume,
    relative: &Path,
    expected: &[u8],
    value: &T,
) -> Result<()> {
    let mut session = Session::open(volume)?;
    let path = session.root.join(relative);
    if relative != Path::new(PROFILE) {
        let dir = path.parent().context("Missing player folder")?;
        let id = session
            .objects
            .get(&object_key(dir))
            .context("Missing player library")?
            .id;
        fs::create_dir_all(dir)?;
        for obj in session.worker.list(session.storage, id)? {
            session
                .objects
                .insert(object_key(&dir.join(&obj.name)), obj);
        }
        session.hydrate(&path)?;
    }
    ensure!(
        read_optional(&path)? == expected,
        "Player settings changed. Refresh and try again."
    );
    let mut file = tempfile::NamedTempFile::new()?;
    serde_json::to_writer_pretty(&mut file, value)?;
    session.put(&path, file.path())
}

pub(crate) fn read_optional(path: &Path) -> Result<Vec<u8>> {
    match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        r => Ok(r?),
    }
}
