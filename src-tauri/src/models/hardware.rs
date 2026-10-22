use sysinfo::System;

#[derive(Debug, Clone, PartialEq)]
pub struct Hardware {
    pub total_ram_gb: f64,
    pub cores: usize,
    pub arch: String,
    pub apple_silicon: bool,
    pub gpu_vendor: Option<String>,
}

impl Hardware {
    pub fn detect() -> Hardware {
        let mut sys = System::new();
        sys.refresh_memory();
        let arch = std::env::consts::ARCH.to_string();
        let apple_silicon = cfg!(target_os = "macos") && arch == "aarch64";
        Hardware {
            total_ram_gb: sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0),
            cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            arch,
            apple_silicon,
            gpu_vendor: apple_silicon.then(|| "Apple".to_string()),
        }
    }
}

const PARAKEET_V3_LANGS: &[&str] = &[
    "bg", "hr", "cs", "da", "nl", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt",
    "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
];
const CJK_LANGS: &[&str] = &["zh", "ja", "ko", "yue"];

/// Pick a catalog model id. Low RAM (<= 4 GB) wins over locale.
pub fn recommend(hw: &Hardware, locale: &str) -> &'static str {
    if hw.total_ram_gb <= 4.0 {
        return "moonshine-base";
    }
    let lang = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if CJK_LANGS.contains(&lang.as_str()) {
        "sensevoice-small"
    } else if PARAKEET_V3_LANGS.contains(&lang.as_str()) {
        "parakeet-v3"
    } else {
        "parakeet-v2"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(ram: f64) -> Hardware {
        Hardware { total_ram_gb: ram, cores: 8, arch: "aarch64".into(), apple_silicon: true, gpu_vendor: None }
    }

    #[test]
    fn recommend_low_ram_is_moonshine() {
        assert_eq!(recommend(&hw(3.5), "en-US"), "moonshine-base");
    }
    #[test]
    fn recommend_exactly_4gb_is_moonshine() {
        assert_eq!(recommend(&hw(4.0), "en-US"), "moonshine-base");
    }
    #[test]
    fn recommend_german_is_parakeet_v3() {
        assert_eq!(recommend(&hw(16.0), "de-DE"), "parakeet-v3");
    }
    #[test]
    fn recommend_japanese_is_sensevoice() {
        assert_eq!(recommend(&hw(16.0), "ja-JP"), "sensevoice-small");
    }
    #[test]
    fn recommend_default_is_parakeet_v2() {
        assert_eq!(recommend(&hw(8.0), "en-GB"), "parakeet-v2");
    }
}
