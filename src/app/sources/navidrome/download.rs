use super::Session;
use std::path::Path;
impl Session {
    /// Download original audio using this library's authenticated transport.
    pub async fn download(
        &self,
        song_id: &str,
        destination: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> anyhow::Result<u64> {
        self.client.download(song_id, destination, progress).await
    }
}
