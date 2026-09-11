use super::*;

fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let root = root.canonicalize().context("Cannot open library root")?;
    let path = root
        .join(relative_path(relative)?)
        .canonicalize()
        .context("Cannot open library entry")?;
    if !path.starts_with(&root) {
        bail!("Symlink leaves the library root");
    }
    Ok(path)
}

pub(super) async fn list(root: PathBuf, relative: String) -> Result<Vec<FolderEntry>> {
    tokio::task::spawn_blocking(move || {
        let path = resolve(&root, &relative)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).context("Cannot read folder")? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Folder contains a non-UTF-8 filename"))?;
            let path = child_path(&relative, &name)?;
            // Never descend through symlinks; this also avoids cycles. A root
            // itself may be a symlink, resolved once per operation.
            let kind = entry.file_type()?;
            if kind.is_dir() || kind.is_file() {
                entries.push(FolderEntry {
                    path,
                    name,
                    directory: kind.is_dir(),
                });
            }
            if entries.len() > MAX_ENTRIES {
                bail!("Folder exceeds {MAX_ENTRIES} entries");
            }
        }
        Ok(entries)
    })
    .await
    .context("Folder worker failed")?
}

pub(super) async fn file(root: PathBuf, relative: String) -> Result<MediaFile> {
    tokio::task::spawn_blocking(move || {
        let path = resolve(&root, &relative)?;
        if !path.is_file() {
            bail!("Selected entry is not a file");
        }
        Ok(MediaFile::local(path))
    })
    .await
    .context("File worker failed")?
}
