//! Atomic, directory-only index. Album file listings remain demand-loaded.
use super::{FolderEntry, FolderSource};
use anyhow::{Context, Result};
use futures::{stream::FuturesUnordered, StreamExt};
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
        let mut requests = FuturesUnordered::new();
        loop {
            while requests.len() < 4 {
                let Some((path, depth)) = pending.pop_front() else {
                    break;
                };
                requests.push(async move {
                    let entries = source.list(&path).await.context("Read folder tree")?;
                    Ok::<_, anyhow::Error>((path, depth, entries))
                });
            }
            let Some(listing) = requests.next().await else {
                break;
            };
            let (path, depth, entries) = listing?;
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

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn completed_branches_expand_while_a_sibling_listing_is_pending() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = FolderSource {
            id: "tree-scheduling-fixture".into(),
            name: "Fixture".into(),
            location: super::super::FolderLocation::Webdav {
                url: format!("http://{}/music/", listener.local_addr().unwrap()),
                username: None,
                password_env: None,
            },
        };
        let leaf_requested = Arc::new(tokio::sync::Notify::new());
        let server = tokio::spawn(async move {
            let mut replies = tokio::task::JoinSet::new();
            for _ in 0..4 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let leaf_requested = leaf_requested.clone();
                replies.spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0; 4096];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&chunk[..n]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    assert!(request.starts_with("PROPFIND "));
                    let path = request.split_whitespace().nth(1).unwrap();
                    let children: &[&str] = match path {
                        "/music/" => &["a/", "b/"],
                        "/music/a/" => &["a/leaf/"],
                        "/music/b/" => {
                            // This sibling cannot finish until the next depth is scheduled.
                            leaf_requested.notified().await;
                            &[]
                        }
                        "/music/a/leaf/" => {
                            leaf_requested.notify_one();
                            &[]
                        }
                        _ => panic!("Unexpected folder"),
                    };
                    let entries = children.iter().map(|child| format!(
                        "<d:response><d:href>/music/{child}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
                    )).collect::<String>();
                    let body = format!("<d:multistatus xmlns:d=\"DAV:\">{entries}</d:multistatus>");
                    socket.write_all(format!("HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                });
            }
            while let Some(reply) = replies.join_next().await {
                reply.unwrap();
            }
        });
        let result = tokio::time::timeout(Duration::from_secs(5), scan(&source)).await;
        if result.is_err() {
            server.abort();
        }
        let tree = result
            .expect("a slow sibling must not block descendant requests")
            .unwrap();
        assert_eq!(tree.directories.len(), 4);
        assert_eq!(tree.listing("a").unwrap()[0].path, "a/leaf");
        server.await.unwrap();
    }
}
