use anyhow::{Context, Result};
use serde::Deserialize;

use super::catalog::{CatalogEntry, EngineKind, ModelFile};

const DEFAULT_BASE: &str = "https://huggingface.co";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HfRepoSummary {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    pub last_modified: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Inspection {
    Compatible(CatalogEntry),
    Incompatible { reason: String },
}

#[derive(Debug, Clone)]
pub struct HfClient {
    base: String,
    token: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct RawSummary {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<String>,
}

#[derive(Deserialize)]
struct RawRepo {
    #[serde(default)]
    sha: String,
    #[serde(default)]
    siblings: Vec<RawSibling>,
}

#[derive(Deserialize)]
struct RawSibling {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<RawLfs>,
}

#[derive(Deserialize)]
struct RawLfs {
    sha256: Option<String>,
}

impl HfClient {
    pub fn new(token: Option<String>) -> Self {
        Self { base: DEFAULT_BASE.to_string(), token: std::sync::Arc::new(std::sync::RwLock::new(token)), http: reqwest::Client::new() }
    }

    /// Replace the bearer token; shared by every clone of this client.
    pub fn set_token(&self, token: Option<String>) {
        *self.token.write().unwrap_or_else(|p| p.into_inner()) = token;
    }

    fn token(&self) -> Option<String> {
        self.token.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Point the client at another server (tests).
    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    /// File URL: `{base}/{repo}/resolve/{revision}/{path}`.
    pub(crate) fn resolve_url(&self, repo: &str, revision: &str, path: &str) -> String {
        format!("{}/{repo}/resolve/{revision}/{path}", self.base)
    }

    /// Authenticated GET builder for an absolute URL (bearer token when set).
    pub(crate) fn authed_get(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.http.get(url);
        match self.token() {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    fn get(&self, url: &str, params: &[(&str, &str)]) -> Result<reqwest::RequestBuilder> {
        let url = reqwest::Url::parse_with_params(url, params)?;
        let req = self.http.get(url);
        Ok(match self.token() {
            Some(t) => req.bearer_auth(t),
            None => req,
        })
    }

    pub async fn search(&self, q: &str) -> Result<Vec<HfRepoSummary>> {
        let url = format!("{}/api/models", self.base);
        let filtered = self.search_once(&url, q, true).await?;
        if !filtered.is_empty() {
            return Ok(filtered);
        }
        self.search_once(&url, q, false).await
    }

    async fn search_once(&self, url: &str, q: &str, filter: bool) -> Result<Vec<HfRepoSummary>> {
        let mut params = vec![("search", q)];
        if filter {
            params.push(("filter", "sherpa-onnx"));
        }
        params.push(("limit", "30"));
        let raw: Vec<RawSummary> = self
            .get(url, &params)?
            .send()
            .await
            .context("hugging face search request failed")?
            .error_for_status()?
            .json()
            .await
            .context("unexpected hugging face search response")?;
        Ok(raw
            .into_iter()
            .map(|r| HfRepoSummary {
                repo: r.id,
                downloads: r.downloads,
                likes: r.likes,
                last_modified: r.last_modified.unwrap_or_default(),
            })
            .collect())
    }

    pub async fn inspect(&self, repo: &str) -> Result<Inspection> {
        let raw: RawRepo = self
            .get(&format!("{}/api/models/{}", self.base, repo), &[("blobs", "true")])?
            .send()
            .await
            .context("hugging face repo request failed")?
            .error_for_status()?
            .json()
            .await
            .context("unexpected hugging face repo response")?;

        let names: Vec<String> = raw.siblings.iter().map(|s| s.rfilename.clone()).collect();
        let Some((engine, chosen)) = detect_engine(&names, repo) else {
            return Ok(Inspection::Incompatible {
                reason: "No supported sherpa-onnx model layout found in this repository.".into(),
            });
        };

        let mut files = Vec::with_capacity(chosen.len());
        for path in chosen {
            let sib = raw.siblings.iter().find(|s| s.rfilename == path);
            let Some(size) = sib.and_then(|s| s.size) else {
                return Ok(Inspection::Incompatible {
                    reason: format!("File size for {path} is not available."),
                });
            };
            let sha256 = sib
                .and_then(|s| s.lfs.as_ref())
                .and_then(|l| l.sha256.clone())
                .unwrap_or_default();
            files.push(ModelFile { path, size, sha256 });
        }

        let total: u64 = files.iter().map(|f| f.size).sum();
        let min_ram_gb = (total * 2).div_ceil(1_000_000_000) as u32;
        Ok(Inspection::Compatible(CatalogEntry {
            id: format!("hf:{repo}"),
            name: repo.rsplit('/').next().unwrap_or(repo).to_string(),
            hf_repo: repo.to_string(),
            revision: raw.sha,
            files,
            engine,
            languages: vec![],
            min_ram_gb,
            notes: "From Hugging Face".into(),
        }))
    }
}

fn is_int8(name: &str) -> bool {
    name.contains("int8")
}

/// Pick the best file among `candidates`: int8 preferred, then shortest name.
fn pick<'a>(candidates: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    candidates.min_by_key(|n| (!is_int8(n), n.len()))
}

/// Detects the engine from a repo's top-level file listing and returns the
/// files to download (int8 preferred). `repo` is used for SenseVoice tagging.
pub fn detect_engine(files: &[String], repo: &str) -> Option<(EngineKind, Vec<String>)> {
    let top: Vec<&String> = files.iter().filter(|f| !f.contains('/')).collect();
    let find = |pred: &dyn Fn(&str) -> bool| pick(top.iter().copied().filter(|f| pred(f)));
    let onnx = |f: &str| f.ends_with(".onnx");

    // Transducer
    if let (Some(e), Some(d), Some(j), Some(t)) = (
        find(&|f| onnx(f) && f.starts_with("encoder")),
        find(&|f| onnx(f) && f.starts_with("decoder")),
        find(&|f| onnx(f) && f.starts_with("joiner")),
        find(&|f| f == "tokens.txt"),
    ) {
        return Some((EngineKind::Transducer, vec![e.clone(), d.clone(), j.clone(), t.clone()]));
    }
    // Whisper
    if let (Some(e), Some(d), Some(t)) = (
        find(&|f| onnx(f) && f.contains("-encoder")),
        find(&|f| onnx(f) && f.contains("-decoder")),
        find(&|f| f.ends_with("-tokens.txt")),
    ) {
        return Some((EngineKind::Whisper, vec![e.clone(), d.clone(), t.clone()]));
    }
    // Moonshine
    if let (Some(p), Some(e), Some(u), Some(c), Some(t)) = (
        find(&|f| f == "preprocess.onnx"),
        find(&|f| onnx(f) && f.starts_with("encode")),
        find(&|f| onnx(f) && f.starts_with("uncached_decode")),
        find(&|f| onnx(f) && f.starts_with("cached_decode")),
        find(&|f| f == "tokens.txt"),
    ) {
        return Some((
            EngineKind::Moonshine,
            vec![p.clone(), e.clone(), u.clone(), c.clone(), t.clone()],
        ));
    }
    // SenseVoice
    let lower = repo.to_lowercase();
    if lower.contains("sense-voice") || lower.contains("sensevoice") {
        if let (Some(m), Some(t)) = (
            find(&|f| onnx(f) && f.starts_with("model")),
            find(&|f| f == "tokens.txt"),
        ) {
            return Some((EngineKind::SenseVoice, vec![m.clone(), t.clone()]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn detects_parakeet_transducer() {
        let files = s(&["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt", "encoder.onnx"]);
        let (kind, mut chosen) = detect_engine(&files, "x/parakeet").unwrap();
        chosen.sort();
        assert_eq!(kind, EngineKind::Transducer);
        assert_eq!(chosen, s(&["decoder.int8.onnx", "encoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]));
    }

    #[test]
    fn detects_whisper() {
        let files = s(&[
            "tiny.en-encoder.onnx", "tiny.en-encoder.int8.onnx", "tiny.en-decoder.onnx",
            "tiny.en-decoder.int8.onnx", "tiny.en-tokens.txt", "test_wavs/0.wav",
        ]);
        let (kind, mut chosen) = detect_engine(&files, "x/whisper").unwrap();
        chosen.sort();
        assert_eq!(kind, EngineKind::Whisper);
        assert_eq!(chosen, s(&["tiny.en-decoder.int8.onnx", "tiny.en-encoder.int8.onnx", "tiny.en-tokens.txt"]));
    }

    #[test]
    fn detects_moonshine() {
        let files = s(&[
            "preprocess.onnx", "encode.int8.onnx", "uncached_decode.int8.onnx",
            "cached_decode.int8.onnx", "tokens.txt", "cached_decode.onnx",
        ]);
        let (kind, mut chosen) = detect_engine(&files, "x/moonshine").unwrap();
        chosen.sort();
        assert_eq!(kind, EngineKind::Moonshine);
        assert_eq!(chosen, s(&[
            "cached_decode.int8.onnx", "encode.int8.onnx", "preprocess.onnx",
            "tokens.txt", "uncached_decode.int8.onnx",
        ]));
    }

    #[test]
    fn detects_sensevoice() {
        let files = s(&["model.int8.onnx", "model.onnx", "tokens.txt"]);
        let (kind, mut chosen) = detect_engine(&files, "x/sherpa-onnx-sense-voice-zh-en").unwrap();
        chosen.sort();
        assert_eq!(kind, EngineKind::SenseVoice);
        assert_eq!(chosen, s(&["model.int8.onnx", "tokens.txt"]));
        assert!(detect_engine(&files, "x/something-else").is_none());
    }

    #[test]
    fn rejects_unknown() {
        assert!(detect_engine(&s(&["pytorch_model.bin", "config.json"]), "x/y").is_none());
    }

    #[tokio::test]
    async fn search_falls_back_without_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/models"))
            .and(query_param("filter", "sherpa-onnx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path("/api/models"))
            .and(query_param("search", "whisper")).and(query_param_is_missing("filter"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "a/whisper-x", "downloads": 5, "likes": 2, "lastModified": "2025-01-01T00:00:00Z"},
                {"id": "a/bare"}
            ])))
            .expect(1).mount(&server).await;
        let out = HfClient::new(None).with_base(&server.uri()).search("whisper").await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].repo, "a/whisper-x");
        assert_eq!(out[0].downloads, 5);
        assert_eq!(out[0].last_modified, "2025-01-01T00:00:00Z");
        assert_eq!(out[1].downloads, 0);
        assert_eq!(out[1].last_modified, "");
    }

    fn repo_json() -> serde_json::Value {
        serde_json::json!({
            "sha": "abc123",
            "siblings": [
                {"rfilename": "encoder.int8.onnx", "size": 1_000_000_000u64, "lfs": {"sha256": "aa"}},
                {"rfilename": "decoder.int8.onnx", "size": 500_000_000u64, "lfs": {"sha256": "bb"}},
                {"rfilename": "joiner.int8.onnx", "size": 100u64, "lfs": {"sha256": "cc"}},
                {"rfilename": "tokens.txt", "size": 10u64},
                {"rfilename": "README.md", "size": 5u64}
            ]
        })
    }

    #[tokio::test]
    async fn inspect_builds_entry_with_sha_from_lfs() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/models/own/my-model"))
            .and(query_param("blobs", "true")).and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(repo_json()))
            .expect(1).mount(&server).await;
        let c = HfClient::new(Some("tok".into())).with_base(&server.uri());
        let Inspection::Compatible(e) = c.inspect("own/my-model").await.unwrap() else { panic!("incompatible") };
        assert_eq!(e.id, "hf:own/my-model");
        assert_eq!(e.name, "my-model");
        assert_eq!(e.hf_repo, "own/my-model");
        assert_eq!(e.revision, "abc123");
        assert_eq!(e.engine, EngineKind::Transducer);
        assert_eq!(e.notes, "From Hugging Face");
        assert!(e.languages.is_empty());
        assert_eq!(e.min_ram_gb, 4); // ceil(1.6e9 * 2 / 1e9) = 4
        let enc = e.files.iter().find(|f| f.path == "encoder.int8.onnx").unwrap();
        assert_eq!((enc.size, enc.sha256.as_str()), (1_000_000_000, "aa"));
        let tok = e.files.iter().find(|f| f.path == "tokens.txt").unwrap();
        assert_eq!((tok.size, tok.sha256.as_str()), (10, ""));
        assert_eq!(e.files.len(), 4);
    }

    #[tokio::test]
    async fn inspect_incompatible_for_unknown_repo() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/models/o/bin"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sha": "z", "siblings": [{"rfilename": "pytorch_model.bin", "size": 1}]
            }))).mount(&server).await;
        let r = HfClient::new(None).with_base(&server.uri()).inspect("o/bin").await.unwrap();
        assert!(matches!(r, Inspection::Incompatible { .. }));
    }
}
