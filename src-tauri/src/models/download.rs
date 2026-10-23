use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::hf::HfClient;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum DlStatus {
    Queued,
    Downloading,
    Paused,
    Verifying,
    Installed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub model_id: String,
    pub bytes_done: u64,
    pub total: u64,
    pub bytes_per_sec: f64,
    pub eta_secs: Option<u64>,
    pub status: DlStatus,
    /// Last error message when auto-retries ran out (status is then `Paused`).
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("checksum mismatch")]
    Checksum,
    #[error("cancelled")]
    Cancelled,
    #[error("http status {0}")]
    Http(u16),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Lets an event through at most once per `interval`.
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last {
            Some(l) if now.duration_since(l) < self.interval => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Download `url` to `dest` via `dest.part`, resuming with a Range request if a
/// partial exists. `on_progress(bytes_so_far)` is throttled to 10 Hz and always
/// called once with the final byte count. Cancellation keeps the `.part`.
/// An empty `expected_sha` skips the checksum and checks length == `total`.
pub async fn download_file(
    client: &HfClient,
    url: &str,
    dest: &Path,
    expected_sha: &str,
    total: u64,
    cancel: CancellationToken,
    on_progress: impl Fn(u64),
) -> Result<(), DownloadError> {
    let part = part_path(dest);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut have = tokio::fs::metadata(&part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if total > 0 && have > total {
        tokio::fs::remove_file(&part).await?;
        have = 0;
    }

    if total == 0 || have < total {
        let mut req = client.authed_get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            r = req.send() => r?,
        };
        let status = resp.status();
        if !status.is_success() {
            return Err(DownloadError::Http(status.as_u16()));
        }
        // 200 to a Range request: server ignored it, restart from scratch.
        let resumed = have > 0 && status.as_u16() == 206;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(!resumed)
            .open(&part)
            .await?;
        if resumed {
            file.seek(SeekFrom::End(0)).await?;
        } else {
            have = 0;
        }
        let mut stream = resp.bytes_stream();
        let mut th = Throttle::new(Duration::from_millis(100));
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    file.flush().await?;
                    return Err(DownloadError::Cancelled);
                }
                n = stream.next() => n,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            have += chunk.len() as u64;
            if th.ready(Instant::now()) {
                on_progress(have);
            }
        }
        file.flush().await?;
        drop(file);
    }
    on_progress(have);

    let ok = if expected_sha.is_empty() {
        total == 0 || have == total
    } else {
        sha256_file(&part).await?.eq_ignore_ascii_case(expected_sha)
    };
    if !ok {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(DownloadError::Checksum);
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}

async fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sha(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn downloads_and_verifies() {
        let server = MockServer::start().await;
        let data = body(300_000);
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let last = std::sync::atomic::AtomicU64::new(0);
        download_file(
            &HfClient::new(None),
            &format!("{}/f.bin", server.uri()),
            &dest,
            &sha(&data),
            data.len() as u64,
            CancellationToken::new(),
            |n| last.store(n, std::sync::atomic::Ordering::SeqCst),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!dir.path().join("f.bin.part").exists());
        assert_eq!(
            last.load(std::sync::atomic::Ordering::SeqCst),
            data.len() as u64
        );
    }

    #[tokio::test]
    async fn resumes_with_range_header() {
        let server = MockServer::start().await;
        let data = body(100_000);
        let half = 50_000;
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .and(header("range", "bytes=50000-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(data[half..].to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        std::fs::write(dir.path().join("f.bin.part"), &data[..half]).unwrap();
        download_file(
            &HfClient::new(None),
            &format!("{}/f.bin", server.uri()),
            &dest,
            &sha(&data),
            data.len() as u64,
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }

    #[tokio::test]
    async fn checksum_mismatch_deletes_part() {
        let server = MockServer::start().await;
        let data = body(10_000);
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let err = download_file(
            &HfClient::new(None),
            &format!("{}/f.bin", server.uri()),
            &dest,
            &"0".repeat(64),
            data.len() as u64,
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Checksum));
        assert!(!dest.exists());
        assert!(!dir.path().join("f.bin.part").exists());
    }

    #[tokio::test]
    async fn empty_sha_verifies_length_instead() {
        let server = MockServer::start().await;
        let data = body(10_000);
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        let url = format!("{}/f.bin", server.uri());
        download_file(
            &HfClient::new(None),
            &url,
            &dest,
            "",
            10_000,
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        let dest2 = dir.path().join("g.bin");
        let err = download_file(
            &HfClient::new(None),
            &url,
            &dest2,
            "",
            9_999,
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Checksum));
    }

    #[test]
    fn progress_throttled_to_10hz() {
        let t0 = Instant::now();
        let mut th = Throttle::new(Duration::from_millis(100));
        // a 1 s transfer producing a chunk every millisecond
        let emitted = (0..=1000u64)
            .filter(|ms| th.ready(t0 + Duration::from_millis(*ms)))
            .count();
        assert!(emitted <= 11, "emitted {emitted}");
        assert!(emitted >= 10, "emitted {emitted}");
    }

    #[tokio::test]
    async fn range_ignored_200_restarts_part() {
        let server = MockServer::start().await;
        let data = body(100_000);
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("f.bin");
        std::fs::write(dir.path().join("f.bin.part"), vec![0xAAu8; 50_000]).unwrap();
        download_file(
            &HfClient::new(None),
            &format!("{}/f.bin", server.uri()),
            &dest,
            &sha(&data),
            data.len() as u64,
            CancellationToken::new(),
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
    }
}
