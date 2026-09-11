//! Library access independent of server, rendering, and audio devices.
//!
//! Folder providers expose only listing and file access. Catalog/recommendation
//! APIs belong alongside these operations, not in a fake server HTTP adapter.

pub mod cache;
pub mod capabilities;
pub mod catalog;
pub mod credentials;
pub mod folder;
mod local;
pub mod models;
pub mod radio;
pub mod station;
pub mod track;
pub mod tree;
mod webdav;
pub use webdav::check as check_webdav;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Stable configuration identity is deliberately separate from display names,
/// paths, credentials, and server-assigned track IDs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderSource {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub location: FolderLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderLocation {
    Local {
        path: PathBuf,
    },
    Webdav {
        url: String,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password_env: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderEntry {
    /// UTF-8 path relative to this source's root, never a URL or server key.
    pub path: String,
    pub name: String,
    pub directory: bool,
}

/// A prepared file is owned until its last consumer (decoder/analysis) drops it.
/// Remote files are private, automatically removed, and never part of the library.
#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub(crate) temporary: Option<Arc<tempfile::TempPath>>,
}
impl MediaFile {
    pub fn local(path: PathBuf) -> Self {
        Self {
            path,
            temporary: None,
        }
    }
    pub(crate) fn temporary(file: tempfile::NamedTempFile) -> Self {
        let temporary = Arc::new(file.into_temp_path());
        Self {
            path: temporary.to_path_buf(),
            temporary: Some(temporary),
        }
    }
    pub fn is_temporary(&self) -> bool {
        self.temporary.is_some()
    }
}

/// Concrete providers share a small contract; no speculative all-purpose
/// backend trait and no server credentials in track metadata or URLs.
impl FolderSource {
    /// Connection identity, independent of display name, ID and secret storage.
    /// Local paths may touch the filesystem; call off the UI thread.
    pub fn same_library(&self, other: &Self) -> bool {
        match (&self.location, &other.location) {
            (FolderLocation::Local { path: a }, FolderLocation::Local { path: b }) => {
                a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
            }
            (
                FolderLocation::Webdav {
                    url: a,
                    username: user_a,
                    ..
                },
                FolderLocation::Webdav {
                    url: b,
                    username: user_b,
                    ..
                },
            ) => {
                user_a.as_deref().unwrap_or("") == user_b.as_deref().unwrap_or("")
                    && matches!((webdav::base_url(a), webdav::base_url(b)), (Ok(a), Ok(b)) if a == b)
            }
            _ => false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            bail!("Source ID must contain only letters, digits, underscores, or hyphens");
        }
        if self.name.trim().is_empty() {
            bail!("Library name is required");
        }
        match &self.location {
            FolderLocation::Local { path } if !path.is_absolute() => {
                bail!("Local folder must be an absolute path")
            }
            FolderLocation::Webdav { url, .. } => {
                webdav::base_url(url)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn kind_name(&self) -> &'static str {
        match self.location {
            FolderLocation::Local { .. } => "Local",
            FolderLocation::Webdav { .. } => "WebDAV",
        }
    }

    pub async fn list(&self, path: &str) -> Result<Vec<FolderEntry>> {
        self.validate()?;
        relative_path(path)?;
        let mut entries = match &self.location {
            FolderLocation::Local { path: root } => {
                local::list(root.clone(), path.to_owned()).await?
            }
            FolderLocation::Webdav { .. } => webdav::list(self, path).await?,
        };
        entries.retain(|e| e.directory || is_audio(&e.name));
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| filename_cmp(&a.name, &b.name))
        });
        Ok(entries)
    }

    pub async fn file(&self, path: &str) -> Result<MediaFile> {
        self.validate()?;
        relative_path(path)?;
        match &self.location {
            FolderLocation::Local { path: root } => {
                local::file(root.clone(), path.to_owned()).await
            }
            FolderLocation::Webdav { .. } => webdav::file(self, path, MAX_REMOTE_BYTES).await,
        }
    }
}

pub(crate) const MAX_ENTRIES: usize = 100_000;
pub(crate) const MAX_REMOTE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) fn relative_path(path: &str) -> Result<&Path> {
    if path.contains(['\\', '\0'])
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Invalid relative library path");
    }
    Ok(Path::new(path))
}

pub(crate) fn child_path(parent: &str, name: &str) -> Result<String> {
    if name.is_empty() || name.contains('/') {
        bail!("Invalid directory entry");
    }
    relative_path(name)?;
    Ok(if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    })
}

pub fn is_audio(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3"
                    | "flac"
                    | "m4a"
                    | "aac"
                    | "ogg"
                    | "oga"
                    | "wav"
                    | "aiff"
                    | "aif"
                    | "alac"
                    | "mp4"
            )
        })
}

/// Natural filename order: disc/track numbers sort numerically without parsing
/// tags or guessing album order. Ties retain a deterministic bytewise order.
pub fn filename_cmp(a: &str, b: &str) -> Ordering {
    let al = a.to_lowercase();
    let bl = b.to_lowercase();
    let (mut aa, mut bb) = (al.as_bytes(), bl.as_bytes());
    while !aa.is_empty() && !bb.is_empty() {
        let order = if aa[0].is_ascii_digit() && bb[0].is_ascii_digit() {
            let an = aa.iter().take_while(|c| c.is_ascii_digit()).count();
            let bn = bb.iter().take_while(|c| c.is_ascii_digit()).count();
            let av = &aa[..an];
            let bv = &bb[..bn];
            let av = &av[av.iter().take_while(|&&c| c == b'0').count()..];
            let bv = &bv[bv.iter().take_while(|&&c| c == b'0').count()..];
            aa = &aa[an..];
            bb = &bb[bn..];
            av.len().cmp(&bv.len()).then_with(|| av.cmp(bv))
        } else {
            let order = aa[0].cmp(&bb[0]);
            aa = &aa[1..];
            bb = &bb[1..];
            order
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    aa.len().cmp(&bb.len()).then_with(|| a.cmp(b))
}

pub(crate) fn source_secret(
    id: &str,
    name: &Option<String>,
) -> Result<Option<zeroize::Zeroizing<String>>> {
    if let Some(key) = name {
        Ok(Some(
            std::env::var(key)
                .map(zeroize::Zeroizing::new)
                .with_context(|| format!("Credential environment variable {key} is not set"))?,
        ))
    } else {
        credentials::load(id)
    }
}
