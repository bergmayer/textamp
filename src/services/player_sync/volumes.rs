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
pub struct Target {
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
            path,
            #[cfg(unix)]
            device: {
                use std::os::unix::fs::MetadataExt;
                metadata.dev()
            },
        })
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
#[cfg(target_os = "macos")]
pub async fn discover() -> Result<Vec<Volume>> {
    // A volume can disappear between enumeration and inspection.
    match discover_once().await {
        Ok(volumes) => Ok(volumes),
        Err(_) => discover_once().await,
    }
}

#[cfg(target_os = "macos")]
async fn discover_once() -> Result<Vec<Volume>> {
    use futures::{StreamExt, stream};
    let list = plist(&["list", "-plist", "external", "physical"]).await?;
    fn visit(value: &serde_json::Value, ids: &mut Vec<String>) {
        if value
            .get("VolumeName")
            .and_then(|value| value.as_str())
            .is_some()
            && let Some(id) = value["DeviceIdentifier"].as_str()
        {
            ids.push(id.into());
        }
        for key in ["AllDisksAndPartitions", "Partitions", "APFSVolumes"] {
            if let Some(children) = value[key].as_array() {
                for child in children {
                    visit(child, ids);
                }
            }
        }
    }
    let mut ids = vec![];
    visit(&list, &mut ids);
    ids.sort();
    ids.dedup();
    let results = stream::iter(ids)
        .map(|id| async move {
            let value = plist(&["info", "-plist", &id]).await?;
            Ok::<_, anyhow::Error>(volume(&value))
        })
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
    let mut volumes = vec![];
    for result in results {
        if let Some(volume) = result? {
            volumes.push(volume);
        }
    }
    volumes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(volumes)
}
#[cfg(not(target_os = "macos"))]
pub async fn discover() -> Result<Vec<Volume>> {
    anyhow::bail!("USB player syncing is available on the Mac")
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
#[cfg(target_os = "macos")]
pub async fn eject(volume: &Volume) -> Result<()> {
    verify(volume).await?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::process::Command::new("/usr/sbin/diskutil")
            .args(["eject", &volume.identifier])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    ensure!(
        output.status.success(),
        "The player is still in use; close its files and try again"
    );
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub async fn eject(_volume: &Volume) -> Result<()> {
    anyhow::bail!("USB player syncing is available on the Mac")
}
