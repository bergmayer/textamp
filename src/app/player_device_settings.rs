//! Portable sync selections stored at the root of a mounted player.
use super::player_sync::{State, WebdavPlayers};
use crate::{
    config::Config,
    library::FolderLocation,
    services::player_sync::{Player, Volume, WebdavSelection},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

const FILE: &str = ".textamp-player.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    volume_uuid: String,
    #[cfg(unix)]
    #[serde(skip)]
    device: u64,
    navidrome: Option<Player>,
    webdav: BTreeMap<String, WebdavSelection>,
    accounts: BTreeMap<String, String>,
}

fn account_key(provider: &str, url: &str, username: &str) -> String {
    let url = reqwest::Url::parse(url)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| url.to_owned());
    format!(
        "{:x}",
        md5::compute(format!(
            "{provider}\n{}\n{username}",
            url.trim_end_matches('/')
        ))
    )
}

pub fn accounts(config: &Config) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    for source in &config.navidrome_sources {
        keys.insert(
            source.id.clone(),
            account_key("navidrome", &source.url, &source.username),
        );
    }
    for source in &config.folder_sources {
        if let FolderLocation::Webdav { url, username, .. } = &source.location {
            keys.insert(
                source.id.clone(),
                account_key("webdav", url, username.as_deref().unwrap_or("")),
            );
        }
    }
    keys
}

impl Profile {
    pub fn empty(&self) -> bool {
        self.navidrome.is_none() && self.webdav.is_empty()
    }
    pub fn read(volume: &Volume) -> Result<Self> {
        #[cfg(unix)]
        let device = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&volume.mount)?.dev()
        };
        let path = volume.mount.join(FILE);
        let meta = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    volume_uuid: volume.uuid.clone(),
                    #[cfg(unix)]
                    device,
                    ..Default::default()
                });
            }
            result => result?,
        };
        ensure!(
            meta.is_file() && meta.len() <= 4 * 1024 * 1024,
            "Invalid player settings file"
        );
        let mut data = Vec::new();
        std::fs::File::open(path)?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut data)?;
        let mut profile: Self =
            serde_json::from_slice(&data).context("Could not read player sync settings")?;
        ensure!(
            profile.volume_uuid == volume.uuid,
            "Player settings belong to a different volume"
        );
        #[cfg(unix)]
        {
            profile.device = device;
        }
        Ok(profile)
    }
    pub fn write(&self, volume: &Volume, previous: &Self) -> Result<()> {
        ensure!(
            Self::read(volume)? == *previous,
            "Player settings changed. Refresh the player and try again."
        );
        let mut temp = tempfile::NamedTempFile::new_in(&volume.mount)?;
        temp.write_all(&serde_json::to_vec_pretty(self)?)?;
        temp.as_file().sync_all()?;
        temp.persist(volume.mount.join(FILE))?;
        Ok(())
    }
    pub fn device_id(&self, local: &str, keys: &BTreeMap<String, String>) -> String {
        if self.accounts.contains_key(local) {
            return local.into();
        }
        keys.get(local)
            .and_then(|key| self.accounts.iter().find(|(_, v)| *v == key))
            .map_or_else(|| local.to_owned(), |(id, _)| id.clone())
    }
    pub fn synced(
        &self,
        volume: &Volume,
    ) -> Result<BTreeMap<String, std::collections::BTreeSet<String>>> {
        self.webdav
            .iter()
            .map(|(id, selection)| {
                Ok((
                    id.clone(),
                    crate::services::player_sync::synced_folders(
                        volume,
                        id,
                        &selection.folders,
                        false,
                    )?,
                ))
            })
            .collect()
    }
    pub fn local_id(&self, device: &str, keys: &BTreeMap<String, String>) -> String {
        if keys.contains_key(device) {
            return device.into();
        }
        self.accounts
            .get(device)
            .and_then(|key| keys.iter().find(|(_, v)| *v == key))
            .map_or_else(|| device.to_owned(), |(id, _)| id.clone())
    }
    pub fn project(
        &self,
        keys: &BTreeMap<String, String>,
        players: &mut Vec<Player>,
        folders: &mut WebdavPlayers,
    ) {
        if let Some(mut p) = self.navidrome.clone() {
            p.source_id = self.local_id(&p.source_id, keys);
            players.push(p);
        }
        folders.insert(
            self.volume_uuid.clone(),
            self.webdav
                .iter()
                .map(|(id, sel)| (self.local_id(id, keys), sel.clone()))
                .collect(),
        );
    }
    pub fn update(&mut self, state: &State, keys: &BTreeMap<String, String>) {
        if let Some(mut p) = state
            .players
            .iter()
            .find(|p| p.volume_uuid == self.volume_uuid)
            .cloned()
        {
            let id = self.device_id(&p.source_id, keys);
            if let Some(key) = keys.get(&p.source_id) {
                self.accounts.insert(id.clone(), key.clone());
            }
            p.source_id = id;
            self.navidrome = Some(p);
        }
        if let Some(folders) = state.webdav.get(&self.volume_uuid) {
            for (local, sel) in folders {
                let id = self.device_id(local, keys);
                if let Some(key) = keys.get(local) {
                    self.accounts.insert(id.clone(), key.clone());
                }
                self.webdav.insert(id, sel.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_follow_the_account_across_computers_with_different_local_ids() {
        let root = tempfile::tempdir().unwrap();
        let volume = Volume {
            uuid: "usb".into(),
            name: "Player".into(),
            mount: root.path().into(),
            identifier: "disk".into(),
            available: 100,
            capacity: 100,
        };
        let old_keys = BTreeMap::from([
            ("nav-a".into(), "nav-account".into()),
            ("dav-a".into(), "share-account".into()),
        ]);
        let new_keys = BTreeMap::from([
            ("nav-b".into(), "nav-account".into()),
            ("dav-b".into(), "share-account".into()),
        ]);
        let blank = Profile::read(&volume).unwrap();
        let mut profile = blank.clone();
        let mut state = State::default();
        state.players.push(Player {
            volume_uuid: "usb".into(),
            name: "Player".into(),
            source_id: "nav-a".into(),
            playlists: vec!["Road Trip".into()],
            auto_sync: false,
        });
        state.webdav.insert(
            "usb".into(),
            BTreeMap::from([(
                "dav-a".into(),
                WebdavSelection {
                    folders: ["Archive".into()].into(),
                    nomedia: true,
                },
            )]),
        );
        profile.update(&state, &old_keys);
        profile.write(&volume, &blank).unwrap();
        let stored = Profile::read(&volume).unwrap();
        let mut other = State::default();
        stored.project(&new_keys, &mut other.players, &mut other.webdav);
        assert_eq!(other.players[0].source_id, "nav-b");
        assert!(!other.players[0].auto_sync);
        assert_eq!(other.players[0].playlists, ["Road Trip"]);
        assert!(other.webdav["usb"]["dav-b"].folders.contains("Archive"));
        assert_eq!(stored.device_id("dav-b", &new_keys), "dav-a");
        other
            .webdav
            .get_mut("usb")
            .unwrap()
            .get_mut("dav-b")
            .unwrap()
            .nomedia = false;
        let mut changed = stored.clone();
        changed.update(&other, &new_keys);
        changed.write(&volume, &stored).unwrap();
        assert!(!Profile::read(&volume).unwrap().webdav["dav-a"].nomedia);
        assert!(profile.write(&volume, &stored).is_err());
    }
}
