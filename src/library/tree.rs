//! Atomic, directory-only index. Album file listings remain demand-loaded.
use super::{FolderEntry, FolderSource};
use anyhow::{Context, Result};
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};

pub const KEY: &str = "folder-tree-v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tree {
    /// Leaves are recorded as empty directory lists, never as cached track lists.
    pub directories: BTreeMap<String, Vec<FolderEntry>>,
    /// Mixed folders need a normal listing so loose tracks are not hidden.
    pub mixed: HashSet<String>,
}
impl Tree {
    pub fn listing(&self, path: &str) -> Option<Vec<FolderEntry>> {
        self.directories
            .get(path)
            .filter(|entries| !entries.is_empty() && !self.mixed.contains(path))
            .cloned()
    }
}

/// Bound depth, directory count, concurrency and total time. A failed scan is
/// never saved by the caller, leaving the last complete tree usable.
pub async fn scan(source: &FolderSource) -> Result<Tree> {
    tokio::time::timeout(std::time::Duration::from_secs(600), async {
        let mut tree = Tree::default();
        let mut pending = VecDeque::from([(String::new(), 0usize)]);
        let mut seen = HashSet::from([String::new()]);
        while !pending.is_empty() {
            let batch: Vec<_> = (0..4).filter_map(|_| pending.pop_front()).collect();
            let listings: Vec<_> = stream::iter(batch)
                .map(|(path, depth)| async move {
                    let entries = source.list(&path).await.context("Read folder tree")?;
                    Ok::<_, anyhow::Error>((path, depth, entries))
                })
                .buffered(4)
                .try_collect()
                .await?;
            for (path, depth, entries) in listings {
                let has_files = entries.iter().any(|entry| !entry.directory);
                let directories: Vec<_> = entries
                    .into_iter()
                    .filter(|entry| entry.directory)
                    .collect();
                if has_files {
                    tree.mixed.insert(path.clone());
                }
                for entry in &directories {
                    anyhow::ensure!(depth < 64, "Folder tree exceeds 64 levels");
                    // Providers confine paths to the root; enforce direct children
                    // here too so aliases/cycles cannot expand the crawl.
                    let parent = entry.path.rsplit_once('/').map_or("", |(parent, _)| parent);
                    anyhow::ensure!(parent == path && entry.path != path, "Invalid child folder");
                    if seen.insert(entry.path.clone()) {
                        anyhow::ensure!(
                            seen.len() <= 100_000,
                            "Folder tree exceeds 100,000 directories"
                        );
                        pending.push_back((entry.path.clone(), depth + 1));
                    }
                }
                tree.directories.insert(path, directories);
            }
        }
        Ok(tree)
    })
    .await
    .context("Folder scan timed out; previous cache retained")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn tree_keeps_directories_not_tracks_and_does_not_hide_loose_tracks() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Artist/Album")).unwrap();
        std::fs::write(root.path().join("Artist/Album/01.flac"), b"not decoded").unwrap();
        std::fs::write(root.path().join("loose.mp3"), b"not decoded").unwrap();
        let source = FolderSource {
            id: "tree-test".into(),
            name: "Tree".into(),
            location: super::super::FolderLocation::Local {
                path: root.path().into(),
            },
        };
        let tree = scan(&source).await.unwrap();
        assert!(tree.listing("").is_none()); // loose track must remain visible
        assert_eq!(tree.listing("Artist").unwrap()[0].path, "Artist/Album");
        assert!(tree.listing("Artist/Album").is_none()); // album loaded on demand
        assert!(tree
            .directories
            .values()
            .flatten()
            .all(|entry| entry.directory));
        assert!(!serde_json::to_string(&tree).unwrap().contains("01.flac"));
    }
}
