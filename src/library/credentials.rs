//! Small private credential store; secrets never enter library config or track IDs.
use crate::util::{private_file::write_private_file, SecretString};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use zeroize::Zeroizing;

static LOCK: Mutex<()> = Mutex::new(());

fn read(path: &Path) -> Result<BTreeMap<String, SecretString>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = Zeroizing::new(std::fs::read_to_string(path)?);
    toml::from_str(&text).map_err(|_| {
        anyhow::anyhow!("Stored library credentials are malformed; the file was left unchanged")
    })
}

pub fn load(id: &str) -> Result<Option<Zeroizing<String>>> {
    load_source("folder", id)
}

pub fn load_source(provider: &str, id: &str) -> Result<Option<Zeroizing<String>>> {
    anyhow::ensure!(
        matches!(provider, "folder" | "navidrome" | "audiomuse"),
        "Invalid credential provider"
    );
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = crate::config::XdgPaths::new("textamp")
        .data_dir
        .join(format!("{provider}-credentials.toml"));
    Ok(read(&path)?
        .remove(id)
        .map(|mut value| Zeroizing::new(value.take())))
}

pub fn save(id: &str, password: SecretString) -> Result<()> {
    save_source("folder", id, password)
}

pub fn save_source(provider: &str, id: &str, password: SecretString) -> Result<()> {
    anyhow::ensure!(
        matches!(provider, "folder" | "navidrome" | "audiomuse"),
        "Invalid credential provider"
    );
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let path = crate::config::XdgPaths::new("textamp")
        .data_dir
        .join(format!("{provider}-credentials.toml"));
    let mut credentials = read(&path)?;
    if password.is_empty() {
        credentials.remove(id);
    } else {
        credentials.insert(id.into(), password);
    }
    let text = Zeroizing::new(toml::to_string(&credentials)?);
    write_private_file(&path, text.as_bytes())?;
    Ok(())
}
