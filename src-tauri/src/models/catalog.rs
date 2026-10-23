use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Transducer,
    Whisper,
    Moonshine,
    SenseVoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub hf_repo: String,
    pub revision: String,
    pub files: Vec<ModelFile>,
    pub engine: EngineKind,
    pub languages: Vec<String>,
    pub min_ram_gb: u32,
    pub notes: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    models: Vec<CatalogEntry>,
}

impl Catalog {
    /// The manifest compiled into the binary.
    pub fn bundled() -> Catalog {
        serde_json::from_str(include_str!("../../resources/catalog.json"))
            .expect("bundled catalog.json is valid")
    }

    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.models.iter().find(|e| e.id == id)
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses_and_has_default() {
        let c = Catalog::bundled();
        let e = c.get("parakeet-v2").expect("default present");
        assert_eq!(e.engine, EngineKind::Transducer);
        assert!(e.files.iter().any(|f| f.path == "encoder.int8.onnx"));
        let mut ids: Vec<&str> = c.entries().iter().map(|e| e.id.as_str()).collect();
        ids.sort();
        let mut want = vec![
            "moonshine-base",
            "parakeet-v2",
            "parakeet-v3",
            "sensevoice-small",
            "whisper-small-en",
            "whisper-tiny-en",
            "whisper-turbo",
        ];
        want.sort();
        assert_eq!(ids, want);
        for entry in c.entries() {
            for f in &entry.files {
                assert!(f.size > 0, "{}", f.path);
                assert_eq!(f.sha256.len(), 64, "{}", f.path);
                assert!(f.sha256.chars().all(|ch| ch.is_ascii_hexdigit()));
            }
        }
    }
}
