//! Mounted-volume discovery and identity checks at the filesystem boundary.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Volume {
    pub uuid: String,
    pub name: String,
    pub mount: PathBuf,
    pub identifier: String,
    pub available: u64,
    pub capacity: u64,
}
impl Volume {
    pub fn is_mtp(&self) -> bool {
        self.identifier.starts_with("mtp:")
    }
}
struct MtpTarget {
    session: std::sync::Mutex<super::mtp::session::Session>,
    volume: Volume,
    source: Option<String>,
}
pub struct Target {
    mtp: Option<std::sync::Arc<MtpTarget>>,
    pub path: PathBuf,
    #[cfg(unix)]
    device: u64,
}
impl Target {
    pub async fn open(path: &Path) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "The player folder is not an absolute path"
        );
        let path = tokio::fs::canonicalize(path).await?;
        let metadata = tokio::fs::metadata(&path).await?;
        ensure!(metadata.is_dir(), "The player is not mounted");
        Ok(Self {
            mtp: None,
            path,
            #[cfg(unix)]
            device: {
                use std::os::unix::fs::MetadataExt;
                metadata.dev()
            },
        })
    }
    pub async fn for_volume(volume: &Volume, source: Option<&str>) -> Result<Self> {
        if !volume.is_mtp() {
            return Self::open(&volume.mount).await;
        }
        let volume = volume.clone();
        let source = source.map(str::to_owned);
        let (session, volume, source) = crate::app::tasks::spawn_blocking(move || {
            let mut s = super::mtp::session::Session::open(&volume)?;
            s.snapshot(source.as_deref())?;
            Ok::<_, anyhow::Error>((s, volume, source))
        })
        .await??;
        let mut target = Self::open(session.work.path()).await?;
        target.mtp = Some(std::sync::Arc::new(MtpTarget {
            session: std::sync::Mutex::new(session),
            volume,
            source,
        }));
        Ok(target)
    }
    async fn object_io(
        &self,
        op: impl Fn(&mut super::mtp::session::Session) -> Result<()> + Send + Sync + 'static,
    ) -> Result<()> {
        let Some(target) = &self.mtp else {
            return Ok(());
        };
        let op = std::sync::Arc::new(op);
        let run = || {
            let target = target.clone();
            let op = op.clone();
            crate::app::tasks::spawn_blocking(move || {
                let mut session = target
                    .session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("MTP session stopped"))?;
                op(&mut session)
            })
        };
        let mut result = run().await?;
        for delay in [1, 2, 4] {
            let Err(error) = &result else { break };
            if !super::mtp::session::reconnectable(error) {
                break;
            }
            tracing::warn!(%error, delay, "Reconnecting MTP player");
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            let target = target.clone();
            result = crate::app::tasks::spawn_blocking(move || {
                target
                    .session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("MTP session stopped"))?
                    .reconnect(&target.volume, target.source.as_deref())
            })
            .await?;
            if result.is_ok() {
                result = run().await?;
            }
        }
        result
    }
    pub(super) async fn hydrate(&self, path: &Path) -> Result<()> {
        let p = path.to_owned();
        self.object_io(move |s| s.hydrate(&p)).await
    }
    pub(super) async fn publish(&self, file: &Path, dest: &Path) -> Result<()> {
        let (f, d) = (file.to_owned(), dest.to_owned());
        self.object_io(move |s| s.put(&d, &f)).await
    }
    pub(super) async fn remove(&self, path: &Path) -> Result<()> {
        let p = path.to_owned();
        self.object_io(move |s| s.remove(&p)).await?;
        match tokio::fs::remove_file(path).await {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => Ok(r?),
        }
    }
    pub async fn check(&self) -> Result<()> {
        let metadata = tokio::fs::metadata(&self.path)
            .await
            .context("The player was disconnected")?;
        ensure!(metadata.is_dir(), "The player was disconnected");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.dev() == self.device,
                "The player was disconnected or replaced"
            );
        }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
async fn plist(arguments: &[&str]) -> Result<serde_json::Value> {
    use std::{process::Stdio, time::Duration};
    use tokio::{io::AsyncWriteExt, process::Command};
    let output = tokio::time::timeout(
        Duration::from_secs(8),
        Command::new("/usr/sbin/diskutil")
            .args(arguments)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!("macOS took too long to check USB storage. Click Refresh to try again.")
    })??;
    ensure!(
        output.status.success(),
        "Could not read player storage information"
    );
    let mut child = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut input = child
        .stdin
        .take()
        .context("Could not read storage information")?;
    input.write_all(&output.stdout).await?;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Could not finish reading USB storage information. Click Refresh to try again."
            )
        })??;
    ensure!(output.status.success(), "Invalid storage information");
    Ok(serde_json::from_slice(&output.stdout)?)
}
#[cfg(target_os = "macos")]
fn volume(value: &serde_json::Value) -> Option<Volume> {
    let mount = value.get("MountPoint")?.as_str()?;
    if value["Internal"].as_bool() != Some(false)
        || value["Writable"].as_bool() == Some(false)
        || value["ReadOnlyVolume"].as_bool() == Some(true)
        || !mount.starts_with("/Volumes/")
    {
        return None;
    }
    Some(Volume {
        uuid: value["VolumeUUID"].as_str()?.into(),
        name: value["VolumeName"].as_str()?.into(),
        mount: mount.into(),
        identifier: value["DeviceIdentifier"].as_str()?.into(),
        available: value["FreeSpace"]
            .as_u64()
            .or_else(|| value["VolumeFreeSpace"].as_u64())
            .or_else(|| value["APFSContainerFree"].as_u64())
            .unwrap_or(0),
        capacity: value["TotalSize"]
            .as_u64()
            .or_else(|| value["VolumeSize"].as_u64())
            .unwrap_or(0),
    })
}
/// Inspect one OS-reported mount or device identifier without mounting or changing it.
#[cfg(target_os = "macos")]
pub async fn inspect(identifier: &str) -> Result<Volume> {
    volume(&plist(&["info", "-plist", identifier]).await?)
        .context("The player is no longer mounted")
}
/// Recheck the OS identity, rather than trusting a saved mount path after reconnection.
#[cfg(target_os = "macos")]
pub async fn verify(expected: &Volume) -> Result<()> {
    if expected.is_mtp() {
        return Ok(());
    } // The object session verifies the on-player identity when it opens.
    let actual = volume(&plist(&["info", "-plist", &expected.identifier]).await?)
        .context("The player is no longer mounted")?;
    ensure!(
        actual.uuid == expected.uuid && actual.mount == expected.mount,
        "The player was disconnected or replaced"
    );
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub async fn verify(_expected: &Volume) -> Result<()> {
    anyhow::bail!("USB player syncing is available on the Mac")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn reconnects_and_resumes_publication_after_partial_upload() {
        let dir = tempfile::tempdir().unwrap();
        let worker = dir.path().join("worker");
        let storage = dir.path().join("storage");
        std::fs::create_dir(&storage).unwrap();
        let uuid = uuid::Uuid::new_v4().to_string();
        std::fs::write(
            storage.join(".textamp-player.json"),
            serde_json::json!({"volume_uuid":uuid}).to_string(),
        )
        .unwrap();
        std::fs::write(
            &worker,
            r#"#!/usr/bin/python3
import json, sys
from pathlib import Path
root = Path(__file__).parent
store = root / "storage"
opens = root / "opens"
generation = int(opens.read_text()) + 1 if opens.exists() else 1
opens.write_text(str(generation))
def handle(path):
    return generation * 100000 + path.stat().st_ino % 100000
for line in sys.stdin:
    q = json.loads(line)
    op = q["op"]
    entries = list(store.iterdir())
    obj = next((p for p in entries if handle(p) == q.get("id")), None)
    if op == "storage":
        reply = dict(device="player",name="card",capacity=1000000,available=900000)
    elif op == "stores":
        reply = [1]
    elif op == "list":
        reply = [dict(id=handle(p),name=p.name,folder=False,size=p.stat().st_size) for p in entries]
    elif op == "read":
        Path(q["path"]).write_bytes(obj.read_bytes())
        reply = True
    elif op == "create":
        path = store / q["name"]
        data = Path(q["path"]).read_bytes()
        if generation == 1 and path.suffix == ".new":
            path.write_bytes(data[:1])
            print(json.dumps(dict(error="Incomplete transfer",reconnect=True)),flush=True)
            sys.exit(2)
        path.write_bytes(data)
        reply = handle(path)
    elif op == "delete":
        obj.unlink()
        reply = True
    elif op == "rename":
        obj.rename(store / q["name"])
        reply = True
    else:
        raise ValueError(op)
    print(json.dumps(reply),flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
        let volume = Volume {
            uuid,
            identifier: "mtp:1:1".into(),
            mount: dir.path().into(),
            name: "Player".into(),
            available: 900000,
            capacity: 1000000,
        };
        std::fs::write(storage.join("track.mp3"), b"old audio").unwrap();
        let session =
            super::super::mtp::session::Session::open_verified(&volume, None, &worker).unwrap();
        let mut target = Target::open(session.work.path()).await.unwrap();
        target.mtp = Some(std::sync::Arc::new(MtpTarget {
            session: std::sync::Mutex::new(session),
            volume,
            source: None,
        }));
        let input = dir.path().join("input");
        std::fs::write(&input, b"complete audio").unwrap();
        target
            .publish(&input, &target.path.join("track.mp3"))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(storage.join("track.mp3")).unwrap(),
            b"complete audio"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("opens")).unwrap(),
            "2"
        );
        assert_eq!(std::fs::read_dir(storage).unwrap().count(), 2);
    }
}
