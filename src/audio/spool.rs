//! Progressive temporary audio storage shared by decoding and later seeks.
use crate::library::MediaFile;
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Transfer {
    length: u64,
    outcome: Outcome,
}
#[derive(Default)]
enum Outcome {
    #[default]
    Receiving,
    Complete,
    Failed(String),
}
pub(crate) struct Spool {
    file: MediaFile,
    transfer: Mutex<Transfer>,
    changed: Condvar,
}
impl Spool {
    pub fn create() -> io::Result<(Arc<Self>, tokio::fs::File)> {
        let temporary = tempfile::NamedTempFile::new()?;
        let writer = tokio::fs::File::from_std(temporary.reopen()?);
        Ok((
            Arc::new(Self {
                file: MediaFile::temporary(temporary),
                transfer: Mutex::default(),
                changed: Condvar::new(),
            }),
            writer,
        ))
    }
    pub fn file(&self) -> MediaFile {
        self.file.clone()
    }
    pub fn published(&self, length: usize) {
        super::lock_or_recover(&self.transfer).length = length as u64;
        self.changed.notify_all();
    }
    pub fn finish(&self, error: Option<String>) {
        let mut transfer = super::lock_or_recover(&self.transfer);
        transfer.outcome = error.map_or(Outcome::Complete, Outcome::Failed);
        self.changed.notify_all();
    }
    pub fn reader(self: &Arc<Self>, cancelled: Arc<AtomicBool>) -> io::Result<Reader> {
        Ok(Reader {
            file: std::fs::File::open(&self.file.path)?,
            spool: self.clone(),
            position: 0,
            cancelled,
        })
    }
}
pub(crate) struct Reader {
    file: std::fs::File,
    spool: Arc<Spool>,
    position: u64,
    cancelled: Arc<AtomicBool>,
}
impl Read for Reader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let mut transfer = super::lock_or_recover(&self.spool.transfer);
        while self.position >= transfer.length && matches!(transfer.outcome, Outcome::Receiving) {
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(0);
            }
            transfer = self
                .spool
                .changed
                .wait_timeout(transfer, Duration::from_millis(10))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Ok(0);
        }
        let available = transfer
            .length
            .saturating_sub(self.position)
            .min(output.len() as u64) as usize;
        if available == 0 {
            return match &transfer.outcome {
                Outcome::Failed(error) => Err(io::Error::other(error.clone())),
                Outcome::Complete | Outcome::Receiving => Ok(0),
            };
        }
        drop(transfer);
        let read = self.file.read(&mut output[..available])?;
        self.position += read as u64;
        Ok(read)
    }
}
impl Seek for Reader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Current(0) => Ok(self.position),
            SeekFrom::Start(0) if self.position == 0 => Ok(0),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "progressive decoding is sequential; completed media supports seeking",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn decoder_reads_before_completion_and_full_file_remains_seekable() {
        use tokio::io::AsyncWriteExt;
        let (spool, mut writer) = Spool::create().unwrap();
        let mut reader = spool.reader(Arc::new(AtomicBool::new(false))).unwrap();
        writer.write_all(b"first").await.unwrap();
        writer.flush().await.unwrap();
        spool.published(5);
        let mut bytes = [0; 5];
        assert_eq!(reader.read(&mut bytes).unwrap(), 5);
        assert_eq!(&bytes, b"first");
        writer.write_all(b"second").await.unwrap();
        writer.flush().await.unwrap();
        spool.published(11);
        spool.finish(None);
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"second");
        let mut complete = std::fs::File::open(&spool.file().path).unwrap();
        complete.seek(SeekFrom::Start(5)).unwrap();
        let mut rest = String::new();
        complete.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "second");
    }
}
