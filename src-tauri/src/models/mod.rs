pub mod catalog;
pub mod download;
pub mod hardware;
pub mod hf;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio_util::sync::CancellationToken;

use crate::store::Store;
use catalog::{Catalog, CatalogEntry};
pub use download::{DlStatus, DownloadError, Progress};
use hf::HfClient;

const ACTIVE_KEY: &str = "active_model";
const MANIFEST: &str = "manifest.json";
const PENDING: &str = "pending.json";
const MAX_RETRIES: u32 = 3;
const PERSIST_EVERY: u64 = 1 << 20;

#[derive(Debug, Clone)]
pub struct InstalledModel {
    pub entry: CatalogEntry,
    pub dir: PathBuf,
}

impl InstalledModel {
    /// First model file whose path ends with / contains `name_suffix`
    /// (exact path match preferred), else `dir/name_suffix`.
    pub fn file(&self, name_suffix: &str) -> PathBuf {
        let f = self
            .entry
            .files
            .iter()
            .find(|f| f.path == name_suffix)
            .or_else(|| {
                self.entry
                    .files
                    .iter()
                    .find(|f| f.path.ends_with(name_suffix))
            })
            .or_else(|| {
                self.entry
                    .files
                    .iter()
                    .find(|f| f.path.contains(name_suffix))
            });
        self.dir
            .join(f.map(|f| f.path.as_str()).unwrap_or(name_suffix))
    }
}

struct Running {
    run_gen: u64,
    token: CancellationToken,
    handle: tauri::async_runtime::JoinHandle<()>,
}

type FreeSpace = Arc<dyn Fn(&Path) -> u64 + Send + Sync>;

#[derive(Clone)]
pub struct Manager {
    root: PathBuf,
    store: Store,
    catalog: Catalog,
    hf: HfClient,
    emit: Arc<dyn Fn(Progress) + Send + Sync>,
    free_space: FreeSpace,
    retry_base: Duration,
    running: Arc<Mutex<HashMap<String, Running>>>,
    next_gen: Arc<std::sync::atomic::AtomicU64>,
}

fn dir_name(id: &str) -> String {
    id.replace(['/', ':', '\\'], "__")
}

/// Model ids become directory names under the models root, so they are untrusted input:
/// non-empty, a strict charset, and a directory name that is neither a separator-bearing
/// path nor made only of dots (`.`, `..`).
pub fn validate_id(id: &str) -> Result<()> {
    let charset_ok = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'));
    let dir = dir_name(id);
    if id.is_empty()
        || id.len() > 200
        || !charset_ok
        || dir.chars().all(|c| c == '.')
        || dir.contains(['/', '\\'])
    {
        return Err(anyhow!("invalid model id {id:?}"));
    }
    Ok(())
}

/// Id plus every file path of an entry; run before anything touches the disk.
pub fn validate_entry(entry: &CatalogEntry) -> Result<()> {
    validate_id(&entry.id)?;
    for f in &entry.files {
        let ok = !f.path.is_empty()
            && Path::new(&f.path)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !ok {
            return Err(anyhow!("Invalid file path: {}", f.path));
        }
    }
    Ok(())
}

fn status_str(s: &DlStatus) -> &'static str {
    match s {
        DlStatus::Queued => "queued",
        DlStatus::Downloading => "downloading",
        DlStatus::Paused => "paused",
        DlStatus::Verifying => "verifying",
        DlStatus::Installed => "installed",
        DlStatus::Failed(_) => "failed",
    }
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

impl Manager {
    pub fn new(
        root: PathBuf,
        store: Store,
        catalog: Catalog,
        hf: HfClient,
        emit: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> Self {
        Self {
            root,
            store,
            catalog,
            hf,
            emit,
            free_space: Arc::new(|p| fs4::available_space(p).unwrap_or(u64::MAX)),
            retry_base: Duration::from_millis(500),
            running: Arc::default(),
            next_gen: Arc::default(),
        }
    }

    /// Replace the free-space probe (tests).
    pub fn with_free_space(mut self, f: FreeSpace) -> Self {
        self.free_space = f;
        self
    }

    /// Base delay of the retry backoff: base, 2*base, 4*base.
    pub fn with_retry_base(mut self, d: Duration) -> Self {
        self.retry_base = d;
        self
    }

    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join(dir_name(id))
    }

    fn read_entry(path: &Path) -> Option<CatalogEntry> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// Start (or restart) downloading `entry`; returns immediately.
    pub fn install(&self, entry: CatalogEntry) {
        if let Err(e) = validate_entry(&entry) {
            // Nothing has been created on disk; surface the problem as a failed download.
            self.report(&entry.id, 0, 0, DlStatus::Failed(e.to_string()), None, 0.0);
            return;
        }
        let id = entry.id.clone();
        // Hold the lock across check, spawn and insert so concurrent installs
        // cannot both spawn and a fast-finishing task cannot beat the insert.
        let mut running = self.running.lock().unwrap();
        if running.contains_key(&id) {
            return;
        }
        let dir = self.model_dir(&id);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join(PENDING),
            serde_json::to_vec(&entry).unwrap_or_default(),
        );
        let run_gen = self
            .next_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let token = CancellationToken::new();
        let this = self.clone();
        let t = token.clone();
        let handle = tauri::async_runtime::spawn(async move {
            this.run_install(entry, t, run_gen).await;
        });
        running.insert(
            id,
            Running {
                run_gen,
                token,
                handle,
            },
        );
    }

    pub fn resume(&self, id: &str) {
        if validate_id(id).is_err() {
            return;
        }
        let dir = self.model_dir(id);
        let entry = Self::read_entry(&dir.join(PENDING)).or_else(|| self.catalog.get(id).cloned());
        if let Some(e) = entry {
            self.install(e);
        }
    }

    async fn stop(&self, id: &str) {
        let r = self.running.lock().unwrap().remove(id);
        if let Some(r) = r {
            r.token.cancel();
            let _ = r.handle.await;
        }
    }

    pub async fn pause(&self, id: &str) {
        if validate_id(id).is_err() {
            return;
        }
        self.stop(id).await;
        let dir = self.model_dir(id);
        let Some(entry) = Self::read_entry(&dir.join(PENDING)) else {
            return;
        };
        let done = self.bytes_on_disk(&entry);
        self.set_status(&entry, done, DlStatus::Paused, None);
    }

    /// Stop and delete partial files and download state.
    pub async fn cancel(&self, id: &str) {
        if validate_id(id).is_err() {
            return;
        }
        self.stop(id).await;
        let dir = self.model_dir(id);
        if !dir.join(MANIFEST).exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Remove an installed (or partial) model and free its disk space.
    pub async fn delete(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        self.stop(id).await;
        let dir = self.model_dir(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        if self.store.get_setting::<String>(ACTIVE_KEY)?.as_deref() == Some(id) {
            self.store.delete_setting(ACTIVE_KEY)?;
            // Fall back to another installed model, if any.
            self.ensure_active();
        }
        Ok(())
    }

    pub fn installed(&self) -> Vec<InstalledModel> {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return vec![];
        };
        let mut v: Vec<InstalledModel> = rd
            .flatten()
            .filter_map(|e| {
                let dir = e.path();
                Self::read_entry(&dir.join(MANIFEST)).map(|entry| InstalledModel { entry, dir })
            })
            .collect();
        v.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
        v
    }

    pub fn active(&self) -> Option<InstalledModel> {
        let id = self
            .store
            .get_setting::<String>(ACTIVE_KEY)
            .ok()
            .flatten()?;
        self.installed().into_iter().find(|m| m.entry.id == id)
    }

    /// The active model, activating the first installed one when the setting is missing, empty or
    /// points at a model that is no longer installed. `None` only when nothing is installed.
    pub fn ensure_active(&self) -> Option<InstalledModel> {
        let installed = self.installed();
        let id = self
            .store
            .get_setting::<String>(ACTIVE_KEY)
            .ok()
            .flatten();
        if let Some(m) = id.and_then(|id| installed.iter().find(|m| m.entry.id == id)) {
            return Some(m.clone());
        }
        let first = installed.into_iter().next()?;
        if let Err(e) = self.store.set_setting(ACTIVE_KEY, &first.entry.id) {
            tracing::warn!("could not persist the active model: {e:#}");
        }
        Some(first)
    }

    pub fn set_active(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        if !self.installed().iter().any(|m| m.entry.id == id) {
            return Err(anyhow!("model {id} is not installed"));
        }
        self.store.set_setting(ACTIVE_KEY, &id.to_string())
    }

    pub fn disk_usage(&self, id: &str) -> u64 {
        if validate_id(id).is_err() {
            return 0;
        }
        dir_size(&self.model_dir(id))
    }

    /// Re-queue downloads interrupted by a restart. Paused ones stay paused.
    pub fn resume_incomplete_on_startup(&self) {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return;
        };
        for e in rd.flatten() {
            let dir = e.path();
            if dir.join(MANIFEST).exists() {
                continue;
            }
            let Some(entry) = Self::read_entry(&dir.join(PENDING)) else {
                continue;
            };
            let rows = self.store.downloads_for(&entry.id).unwrap_or_default();
            let paused = rows
                .iter()
                .any(|r| r.status == "paused" || r.status == "failed");
            if paused {
                let total = entry.files.iter().map(|f| f.size).sum();
                let done = self.bytes_on_disk(&entry);
                self.report(&entry.id, done, total, DlStatus::Paused, None, 0.0);
            } else {
                self.install(entry);
            }
        }
    }

    fn bytes_on_disk(&self, entry: &CatalogEntry) -> u64 {
        let dir = self.model_dir(&entry.id);
        entry
            .files
            .iter()
            .map(|f| {
                let p = dir.join(&f.path);
                let mut part = p.clone().into_os_string();
                part.push(".part");
                std::fs::metadata(&p)
                    .or_else(|_| std::fs::metadata(PathBuf::from(part)))
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .sum()
    }

    fn report(
        &self,
        id: &str,
        done: u64,
        total: u64,
        status: DlStatus,
        error: Option<String>,
        bps: f64,
    ) {
        let eta = (bps > 1.0 && total > done).then(|| ((total - done) as f64 / bps).ceil() as u64);
        (self.emit)(Progress {
            model_id: id.to_string(),
            bytes_done: done,
            total,
            bytes_per_sec: bps,
            eta_secs: eta,
            status,
            error,
        });
    }

    /// Emit + persist a status change for every file of the entry.
    fn set_status(&self, entry: &CatalogEntry, done: u64, status: DlStatus, error: Option<String>) {
        let total: u64 = entry.files.iter().map(|f| f.size).sum();
        let s = status_str(&status);
        for f in &entry.files {
            let (d, t) = (self.file_done(entry, f), f.size as i64);
            let _ = self
                .store
                .download_upsert(&entry.id, &f.path, d as i64, t, s);
        }
        self.report(&entry.id, done, total, status, error, 0.0);
    }

    fn file_done(&self, entry: &CatalogEntry, f: &catalog::ModelFile) -> u64 {
        let p = self.model_dir(&entry.id).join(&f.path);
        let mut part = p.clone().into_os_string();
        part.push(".part");
        std::fs::metadata(&p)
            .or_else(|_| std::fs::metadata(PathBuf::from(part)))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    fn finish(&self, id: &str, run_gen: u64) {
        let mut r = self.running.lock().unwrap();
        if r.get(id).map(|x| x.run_gen) == Some(run_gen) {
            r.remove(id);
        }
    }

    /// Runs the install; the slot is released before the terminal status is
    /// emitted so a listener reacting to it can immediately retry.
    async fn run_install(&self, entry: CatalogEntry, token: CancellationToken, run_gen: u64) {
        match self.install_inner(&entry, token).await {
            None => {} // cancelled: pause()/cancel() own the final state
            Some((done, status, err)) => {
                self.finish(&entry.id, run_gen);
                if status == DlStatus::Installed {
                    // Before the event goes out, so listeners already see the active model.
                    self.ensure_active();
                }
                self.set_status(&entry, done, status, err);
            }
        }
    }

    async fn install_inner(
        &self,
        entry: &CatalogEntry,
        token: CancellationToken,
    ) -> Option<(u64, DlStatus, Option<String>)> {
        let entry = entry.clone();
        let id = entry.id.clone();
        for f in &entry.files {
            let ok = Path::new(&f.path)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
            if !ok || f.path.is_empty() {
                return Some((
                    0,
                    DlStatus::Failed(format!("Invalid file path: {}", f.path)),
                    None,
                ));
            }
        }
        let dir = self.model_dir(&id);
        let total: u64 = entry.files.iter().map(|f| f.size).sum();
        self.set_status(&entry, self.bytes_on_disk(&entry), DlStatus::Queued, None);

        let remaining = total.saturating_sub(self.bytes_on_disk(&entry));
        let free = (self.free_space)(&self.root);
        if free < remaining {
            let msg = format!(
                "Not enough disk space: need {} MB, {} MB free",
                remaining >> 20,
                free >> 20
            );
            return Some((0, DlStatus::Failed(msg), None));
        }

        let started = Instant::now();
        let start_bytes = self.bytes_on_disk(&entry);
        let mut base_done = 0u64;
        for f in &entry.files {
            let dest = dir.join(&f.path);
            if std::fs::metadata(&dest)
                .map(|m| m.len() == f.size)
                .unwrap_or(false)
            {
                base_done += f.size;
                continue;
            }
            let url = self
                .hf
                .resolve_url(&entry.hf_repo, &entry.revision, &f.path);
            let mut attempt = 0u32;
            loop {
                let cb = {
                    let this = self.clone();
                    let entry = entry.clone();
                    let (size, path) = (f.size, f.path.clone());
                    let last_persist = Mutex::new(0u64);
                    move |n: u64| {
                        let done = base_done + n;
                        let status = if n >= size && size > 0 {
                            DlStatus::Verifying
                        } else {
                            DlStatus::Downloading
                        };
                        let secs = started.elapsed().as_secs_f64().max(0.001);
                        let bps = done.saturating_sub(start_bytes) as f64 / secs;
                        let mut lp = last_persist.lock().unwrap();
                        if n.saturating_sub(*lp) >= PERSIST_EVERY || n >= size {
                            *lp = n;
                            let _ = this.store.download_upsert(
                                &entry.id,
                                &path,
                                n as i64,
                                size as i64,
                                status_str(&status),
                            );
                        }
                        let total: u64 = entry.files.iter().map(|f| f.size).sum();
                        this.report(&entry.id, done, total, status, None, bps);
                    }
                };
                self.set_status(
                    &entry,
                    base_done + self.file_done(&entry, f),
                    DlStatus::Downloading,
                    None,
                );
                let res = download::download_file(
                    &self.hf,
                    &url,
                    &dest,
                    &f.sha256,
                    f.size,
                    token.clone(),
                    cb,
                )
                .await;
                match res {
                    Ok(()) => break,
                    Err(DownloadError::Cancelled) => return None,
                    Err(DownloadError::Checksum) => {
                        return Some((0, DlStatus::Failed("Checksum mismatch".into()), None));
                    }
                    Err(e) if attempt >= MAX_RETRIES => {
                        let done = base_done + self.file_done(&entry, f);
                        return Some((done, DlStatus::Paused, Some(e.to_string())));
                    }
                    Err(_) => {
                        let delay = self.retry_base * 2u32.pow(attempt);
                        attempt += 1;
                        tokio::select! {
                            _ = token.cancelled() => return None,
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
            }
            base_done += f.size;
        }

        // Manifest is written last: its presence means "installed".
        let tmp = dir.join("manifest.json.tmp");
        let written = serde_json::to_vec_pretty(&entry)
            .map_err(anyhow::Error::from)
            .and_then(|b| std::fs::write(&tmp, b).map_err(Into::into))
            .and_then(|_| std::fs::rename(&tmp, dir.join(MANIFEST)).map_err(Into::into));
        if let Err(e) = written {
            return Some((total, DlStatus::Failed(e.to_string()), None));
        }
        let _ = std::fs::remove_file(dir.join(PENDING));
        Some((total, DlStatus::Installed, None))
    }
}

#[cfg(test)]
mod tests {
    use super::catalog::{CatalogEntry, EngineKind, ModelFile};
    use super::download::{DlStatus, Progress};
    use super::hf::HfClient;
    use super::*;
    use crate::store::Store;
    use sha2::{Digest, Sha256};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sha(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    struct Fx {
        server: MockServer,
        entry: CatalogEntry,
        events: Arc<Mutex<Vec<Progress>>>,
        store: Store,
        files: Vec<(String, Vec<u8>)>,
    }

    async fn fixture() -> Fx {
        let server = MockServer::start().await;
        let files: Vec<(String, Vec<u8>)> = vec![
            (
                "encoder.onnx".into(),
                (0..40_000).map(|i| (i % 200) as u8).collect(),
            ),
            ("tokens.txt".into(), b"a 0\nb 1\n".to_vec()),
        ];
        let entry = CatalogEntry {
            id: "hf:owner/name".into(),
            name: "Test".into(),
            hf_repo: "owner/name".into(),
            revision: "main".into(),
            files: files
                .iter()
                .map(|(p, d)| ModelFile {
                    path: p.clone(),
                    size: d.len() as u64,
                    sha256: sha(d),
                })
                .collect(),
            engine: EngineKind::Transducer,
            languages: vec!["en".into()],
            min_ram_gb: 1,
            notes: String::new(),
        };
        Fx {
            server,
            entry,
            events: Arc::default(),
            store: Store::open_in_memory().unwrap(),
            files,
        }
    }

    async fn mount_ok(fx: &Fx) {
        for (p, d) in &fx.files {
            Mock::given(method("GET"))
                .and(path(format!("/owner/name/resolve/main/{p}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(d.clone()))
                .mount(&fx.server)
                .await;
        }
    }

    fn manager(fx: &Fx, root: &Path) -> Manager {
        let ev = fx.events.clone();
        Manager::new(
            root.to_path_buf(),
            fx.store.clone(),
            super::catalog::Catalog::bundled(),
            HfClient::new(None).with_base(&fx.server.uri()),
            Arc::new(move |p| ev.lock().unwrap().push(p)),
        )
        .with_free_space(Arc::new(|_| u64::MAX))
        .with_retry_base(Duration::from_millis(1))
    }

    async fn wait_for(fx: &Fx, pred: impl Fn(&Progress) -> bool) -> Progress {
        for _ in 0..300 {
            if let Some(p) = fx.events.lock().unwrap().iter().rev().find(|p| pred(p)) {
                return p.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timeout; events: {:?}", fx.events.lock().unwrap().last());
    }

    #[tokio::test]
    async fn manager_marks_installed_and_lists() {
        let fx = fixture().await;
        mount_ok(&fx).await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        assert!(m.installed().is_empty());
        m.install(fx.entry.clone());
        let p = wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        assert_eq!(p.model_id, "hf:owner/name");
        let list = m.installed();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].entry.id, "hf:owner/name");
        assert_eq!(
            std::fs::read(list[0].file("tokens")).unwrap(),
            fx.files[1].1
        );
        assert!(m.disk_usage("hf:owner/name") >= 40_000);
        // The first installed model is activated automatically.
        assert_eq!(m.active().unwrap().entry.id, "hf:owner/name");
        m.set_active("hf:owner/name").unwrap();
        assert_eq!(m.active().unwrap().entry.id, "hf:owner/name");
        m.delete("hf:owner/name").await.unwrap();
        assert!(m.installed().is_empty());
        assert!(m.active().is_none());
    }

    #[tokio::test]
    async fn set_active_requires_installed() {
        let fx = fixture().await;
        let root = tempfile::tempdir().unwrap();
        assert!(manager(&fx, root.path()).set_active("nope").is_err());
    }

    #[tokio::test]
    async fn installs_into_dir_with_space_and_unicode() {
        let fx = fixture().await;
        mount_ok(&fx).await;
        let root = tempfile::tempdir().unwrap();
        let root = root.path().join("My Models ñ");
        let m = manager(&fx, &root);
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        let f = m.installed()[0].file("encoder");
        assert!(f.starts_with(&root) && f.exists());
    }

    #[tokio::test]
    async fn retries_three_times_then_paused() {
        let fx = fixture().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(4)
            .mount(&fx.server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        let p = wait_for(&fx, |p| p.status == DlStatus::Paused).await;
        assert!(p.error.is_some());
        fx.server.verify().await;
    }

    #[tokio::test]
    async fn cancel_stops_and_removes_part() {
        let fx = fixture().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_bytes(vec![0u8; 10]),
            )
            .mount(&fx.server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        let dir = m.model_dir("hf:owner/name");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("encoder.onnx.part"), b"partial").unwrap();
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Downloading).await;
        m.cancel("hf:owner/name").await;
        assert!(!dir.exists());
        assert!(m.installed().is_empty());
    }

    #[tokio::test]
    async fn insufficient_disk_space_fails_early() {
        let fx = fixture().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&fx.server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path()).with_free_space(Arc::new(|_| 1));
        m.install(fx.entry.clone());
        let p = wait_for(&fx, |p| matches!(p.status, DlStatus::Failed(_))).await;
        assert!(matches!(p.status, DlStatus::Failed(ref s) if s.contains("disk")));
        fx.server.verify().await;
    }

    #[tokio::test]
    async fn resume_incomplete_on_startup_restarts_and_keeps_paused() {
        let fx = fixture().await;
        let root = tempfile::tempdir().unwrap();
        // 1) a failing install ends Paused; a fresh manager must not restart it
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&fx.server)
            .await;
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Paused).await;
        fx.server.reset().await;
        mount_ok(&fx).await;
        fx.events.lock().unwrap().clear();
        let m2 = manager(&fx, root.path());
        m2.resume_incomplete_on_startup();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(m2.installed().is_empty());
        // 2) once the row says Downloading (crash mid-download) it restarts
        for (p, d) in &fx.files {
            fx.store
                .download_upsert("hf:owner/name", p, 10, d.len() as i64, "downloading")
                .unwrap();
        }
        let m3 = manager(&fx, root.path());
        m3.resume_incomplete_on_startup();
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        assert_eq!(m3.installed().len(), 1);
    }

    #[tokio::test]
    async fn user_pause_survives_restart() {
        let fx = fixture().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_bytes(vec![0u8; 10]),
            )
            .mount(&fx.server)
            .await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Downloading).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        m.pause("hf:owner/name").await;
        let before = fx.server.received_requests().await.unwrap().len();
        fx.events.lock().unwrap().clear();
        let m2 = manager(&fx, root.path());
        m2.resume_incomplete_on_startup();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(fx.server.received_requests().await.unwrap().len(), before);
        assert_eq!(
            fx.events.lock().unwrap().last().unwrap().status,
            DlStatus::Paused
        );
    }

    #[tokio::test]
    async fn install_after_fast_failure_can_retry() {
        let fx = fixture().await;
        mount_ok(&fx).await;
        let root = tempfile::tempdir().unwrap();
        let free = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let f2 = free.clone();
        let m = manager(&fx, root.path()).with_free_space(Arc::new(move |_| {
            f2.load(std::sync::atomic::Ordering::SeqCst)
        }));
        m.install(fx.entry.clone());
        wait_for(&fx, |p| matches!(p.status, DlStatus::Failed(_))).await;
        free.store(u64::MAX, std::sync::atomic::Ordering::SeqCst);
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
    }

    #[tokio::test]
    async fn double_install_spawns_once() {
        let fx = fixture().await;
        for (p, d) in &fx.files {
            Mock::given(method("GET"))
                .and(path(format!("/owner/name/resolve/main/{p}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(d.clone()))
                .expect(1)
                .mount(&fx.server)
                .await;
        }
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        fx.server.verify().await;
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let mut fx = fixture().await;
        fx.entry.files[0].path = "../evil.onnx".into();
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| matches!(p.status, DlStatus::Failed(_))).await;
        assert!(!root.path().join("evil.onnx").exists());
    }

    #[tokio::test]
    async fn rejects_unsafe_model_ids_everywhere() {
        let fx = fixture().await;
        let root = tempfile::tempdir().unwrap();
        let models = root.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        // Something a traversal would reach: a sibling of the models dir.
        std::fs::write(root.path().join("precious.txt"), b"x").unwrap();
        let m = manager(&fx, &models);
        for bad in ["..", "", ".", "...", "a b", "a\\b", "x\0y", "é"] {
            assert!(m.delete(bad).await.is_err(), "delete({bad:?}) must fail");
            assert!(m.set_active(bad).is_err(), "set_active({bad:?}) must fail");
        }
        assert!(root.path().join("precious.txt").exists());
        assert!(models.exists());
        for good in ["parakeet-v2", "hf:owner/name", "a.b_c-d"] {
            assert!(validate_id(good).is_ok(), "{good}");
        }
    }

    #[tokio::test]
    async fn install_with_unsafe_id_or_path_creates_nothing_and_fails() {
        let mut fx = fixture().await;
        let root = tempfile::tempdir().unwrap();
        let models = root.path().join("models");
        let m = manager(&fx, &models);
        fx.entry.id = "..".into();
        m.install(fx.entry.clone());
        wait_for(&fx, |p| matches!(p.status, DlStatus::Failed(_))).await;
        fx.events.lock().unwrap().clear();
        fx.entry.id = "hf:owner/name".into();
        fx.entry.files[0].path = "../evil.onnx".into();
        m.install(fx.entry.clone());
        wait_for(&fx, |p| matches!(p.status, DlStatus::Failed(_))).await;
        // Validation happens before the model dir or pending.json is written.
        assert!(!m.model_dir("hf:owner/name").exists());
        assert!(!models.exists() || std::fs::read_dir(&models).unwrap().count() == 0);
    }

    #[tokio::test]
    async fn first_installed_model_becomes_active() {
        let fx = fixture().await;
        mount_ok(&fx).await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        assert!(m.active().is_none());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        assert_eq!(m.active().unwrap().entry.id, "hf:owner/name");
    }

    #[tokio::test]
    async fn ensure_active_repairs_missing_or_stale_setting() {
        let fx = fixture().await;
        mount_ok(&fx).await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        m.install(fx.entry.clone());
        wait_for(&fx, |p| p.status == DlStatus::Installed).await;
        // Legacy "" value and a dangling id both count as "no valid active model".
        for stale in ["", "gone"] {
            fx.store.set_setting(ACTIVE_KEY, &stale.to_string()).unwrap();
            assert_eq!(m.ensure_active().unwrap().entry.id, "hf:owner/name");
            assert_eq!(
                fx.store.get_setting::<String>(ACTIVE_KEY).unwrap().as_deref(),
                Some("hf:owner/name")
            );
        }
    }

    #[tokio::test]
    async fn deleting_the_active_model_clears_setting_and_activates_another() {
        let fx = fixture().await;
        let root = tempfile::tempdir().unwrap();
        let m = manager(&fx, root.path());
        for id in ["a-model", "b-model"] {
            let dir = m.model_dir(id);
            std::fs::create_dir_all(&dir).unwrap();
            let mut e = fx.entry.clone();
            e.id = id.into();
            std::fs::write(dir.join(MANIFEST), serde_json::to_vec(&e).unwrap()).unwrap();
        }
        m.set_active("a-model").unwrap();
        m.delete("a-model").await.unwrap();
        assert_eq!(m.active().unwrap().entry.id, "b-model");
        m.delete("b-model").await.unwrap();
        assert!(m.active().is_none());
        // The key is removed, not stored as "".
        assert_eq!(fx.store.get_setting::<String>(ACTIVE_KEY).unwrap(), None);
    }
}
