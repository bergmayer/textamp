use super::Client;
use futures::StreamExt;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncWriteExt, BufWriter};

impl Client {
    /// Download original audio to a private staging file, reporting bounded progress.
    /// # Errors
    /// Returns HTTP, idle-timeout, truncated-body, and filesystem errors.
    pub async fn download(
        &self,
        song_id: &str,
        destination: &Path,
        progress: impl FnMut(u64, Option<u64>),
    ) -> anyhow::Result<u64> {
        self.download_to(song_id, destination, progress, None).await
    }

    pub(crate) async fn download_to(
        &self,
        song_id: &str,
        destination: &Path,
        mut progress: impl FnMut(u64, Option<u64>),
        mut digest: Option<&mut md5::Context>,
    ) -> anyhow::Result<u64> {
        let url = self.url("download", &[("id", song_id.to_owned())])?;
        let response = tokio::time::timeout(Duration::from_secs(30), self.http().get(url).send())
            .await
            .map_err(|_| {
                anyhow::anyhow!("The server did not begin the audio transfer within 30 seconds")
            })?
            .map_err(|error| error.without_url())?
            .error_for_status()
            .map_err(|error| error.without_url())?;
        anyhow::ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Audio transfer returned HTTP {}",
            response.status()
        );
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        anyhow::ensure!(
            !matches!(
                content_type.as_str(),
                "application/json"
                    | "text/json"
                    | "application/xml"
                    | "text/xml"
                    | "text/html"
                    | "application/xhtml+xml"
            ),
            "The server returned a document instead of audio"
        );
        let total = response.content_length();
        let mut received = 0;
        let file = tokio::fs::File::create(destination).await?;
        let mut output = BufWriter::with_capacity(1024 * 1024, file);
        let mut chunks = response.bytes_stream();
        let mut reported = Instant::now();
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), chunks.next()).await?
        {
            let chunk = chunk.map_err(|error| error.without_url())?;
            if let Some(hash) = digest.as_mut() {
                hash.consume(&chunk);
            }
            output.write_all(&chunk).await?;
            received += chunk.len() as u64;
            if reported.elapsed() >= Duration::from_millis(250) {
                progress(received, total);
                reported = Instant::now();
            }
        }
        output.flush().await?;
        output.get_ref().sync_all().await?;
        anyhow::ensure!(received > 0, "The server returned an empty audio file");
        anyhow::ensure!(
            total.is_none_or(|total| total == received),
            "Incomplete download"
        );
        Ok(received)
    }
}
