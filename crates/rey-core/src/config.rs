use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use url::{Host, Url};

#[derive(Clone)]
pub struct Config {
    pub root: PathBuf,
    pub database: PathBuf,
    pub legacy_database: PathBuf,
    pub provider: ProviderKind,
    pub ollama: Url,
    pub llama_cpp: Url,
    pub llama_cpp_key: Option<String>,
    pub llama_cpp_key_file: Option<PathBuf>,
    pub ollama_model: String,
    pub llama_cpp_model: String,
    pub timeout: Duration,
    pub history: usize,
    pub memory_enabled: bool,
    pub remote_memory: bool,
    pub child_mode: String,
    pub voice_enabled: bool,
    pub python: PathBuf,
    pub voice_url: Option<Url>,
    pub voice_token: Option<String>,
    pub voice_output: PathBuf,
    pub keep_audio: bool,
    pub unload_ollama: bool,
    pub max_audio_bytes: u64,
    pub max_audio_seconds: u32,
    pub chess_engine: PathBuf,
    pub shogi_engine: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Ollama,
    LlamaCpp,
}

impl ProviderKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LlamaCpp => "llama.cpp",
        }
    }
}

impl Config {
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = std::fs::canonicalize(root).context("Каталог проекта недоступен")?;
        let path = root.join(".env");
        let mut values = HashMap::new();
        if path.is_file() {
            for entry in dotenvy::from_path_iter(&path).context("Не удалось прочитать .env")?
            {
                let (key, value) = entry.map_err(|_| anyhow::anyhow!("Ошибка формата .env"))?;
                values.insert(key, value);
            }
        }
        Self::from_lookup(root, |key| {
            std::env::var(key).ok().or_else(|| values.get(key).cloned())
        })
    }

    pub fn provider_url(&self) -> &Url {
        match self.provider {
            ProviderKind::Ollama => &self.ollama,
            ProviderKind::LlamaCpp => &self.llama_cpp,
        }
    }

    pub fn set_provider_url(&mut self, url: Url) {
        match self.provider {
            ProviderKind::Ollama => self.ollama = url,
            ProviderKind::LlamaCpp => self.llama_cpp = url,
        }
    }

    pub fn model(&self) -> &str {
        match self.provider {
            ProviderKind::Ollama => &self.ollama_model,
            ProviderKind::LlamaCpp => &self.llama_cpp_model,
        }
    }

    pub fn set_model(&mut self, model: String) {
        match self.provider {
            ProviderKind::Ollama => self.ollama_model = model,
            ProviderKind::LlamaCpp => self.llama_cpp_model = model,
        }
    }

    fn from_lookup(root: PathBuf, lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key, default: &str| lookup(key).unwrap_or_else(|| default.to_string());
        let flag = |key, default| {
            lookup(key).map_or(default, |value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on" | "y"
                )
            })
        };
        let limit = |key, default: u64, min, max| -> Result<u64> {
            let value = match lookup(key) {
                Some(value) => value
                    .parse()
                    .with_context(|| format!("Ожидалось число в {key}"))?,
                None => default,
            };
            ensure!(
                (min..=max).contains(&value),
                "{key} вне диапазона {min}–{max}"
            );
            Ok(value)
        };
        let resolved = |key, default| resolve(&root, &get(key, default));
        let executable = |key, default: &str| {
            let value = PathBuf::from(get(key, default));
            if value.components().count() > 1 || root.join(&value).is_file() {
                root.join(value)
            } else {
                value
            }
        };
        let game_engine = |key, name: &str| {
            let value = executable(key, name);
            let binary = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            };
            let local = root.join("data/engines").join(binary);
            if value == Path::new(name) && local.is_file() {
                local
            } else {
                value
            }
        };
        let provider = match get("REY_LLM_PROVIDER", "ollama")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "ollama" => ProviderKind::Ollama,
            "llama_cpp" | "llama.cpp" => ProviderKind::LlamaCpp,
            _ => anyhow::bail!("REY_LLM_PROVIDER должен быть ollama или llama_cpp"),
        };
        let legacy_url = get("DATABASE_URL", "sqlite:///data/rei_memory.sqlite3");
        let legacy_database = root.join(
            legacy_url
                .strip_prefix("sqlite:///")
                .filter(|value| !value.is_empty())
                .context("DATABASE_URL должен иметь вид sqlite:///путь")?,
        );
        let voice_url = lookup("REY_VOICE_URL")
            .filter(|value| !value.is_empty())
            .map(|value| endpoint(&value))
            .transpose()?;
        let voice_token = lookup("REY_VOICE_TOKEN").filter(|value| !value.is_empty());
        if let Some(url) = &voice_url {
            ensure!(
                is_local(url) || (url.scheme() == "https" && voice_token.is_some()),
                "Удалённый голосовой провайдер требует HTTPS и REY_VOICE_TOKEN"
            );
        }
        let python_default = if cfg!(windows) {
            ".venv/Scripts/python.exe"
        } else {
            ".venv/bin/python"
        };
        let python = if root.join(python_default).is_file() {
            python_default
        } else if cfg!(windows) {
            "python"
        } else {
            "python3"
        };
        Ok(Self {
            database: resolved("REY_DATABASE", "ghost/memory.db"),
            legacy_database,
            ollama: endpoint(&get("OLLAMA_HOST", "http://127.0.0.1:11434"))?,
            provider,
            llama_cpp: endpoint(&get("REY_LLAMA_CPP_HOST", "http://127.0.0.1:8080"))?,
            llama_cpp_key: lookup("REY_LLAMA_CPP_API_KEY").filter(|key| !key.trim().is_empty()),
            llama_cpp_key_file: lookup("REY_LLAMA_CPP_API_KEY_FILE")
                .filter(|path| !path.trim().is_empty())
                .map(|path| root.join(path)),
            ollama_model: get("OLLAMA_MODEL", "qwen2.5:7b-instruct"),
            llama_cpp_model: get("REY_LLAMA_CPP_MODEL", ""),
            timeout: Duration::from_secs(limit("OLLAMA_TIMEOUT_SECONDS", 90, 1, 3600)?),
            history: limit("MAX_HISTORY_MESSAGES", 20, 1, 200)? as usize,
            memory_enabled: flag("MEMORY_ENABLED", true),
            remote_memory: flag("REY_ALLOW_REMOTE_MEMORY", false),
            child_mode: get("CHILD_SAFE_MODE", "auto"),
            voice_enabled: flag("VOICE_ENABLED", false),
            python: executable("REY_PYTHON", python),
            voice_url,
            voice_token,
            voice_output: resolved("REY_VOICE_OUTPUT_DIR", "data/voice_output"),
            keep_audio: flag("REY_KEEP_OUTPUT_AUDIO", true),
            unload_ollama: flag("REY_OLLAMA_UNLOAD_BEFORE_TTS", true),
            max_audio_bytes: limit(
                "REY_MAX_AUDIO_BYTES",
                15 * 1024 * 1024,
                1024,
                64 * 1024 * 1024,
            )?,
            max_audio_seconds: limit("REY_MAX_AUDIO_SECONDS", 180, 1, 600)? as u32,
            chess_engine: game_engine("REY_CHESS_ENGINE", "stockfish"),
            shogi_engine: game_engine("REY_SHOGI_ENGINE", "YaneuraOu"),
            root,
        })
    }
}

fn resolve(root: &Path, value: &str) -> PathBuf {
    root.join(value)
}

pub fn endpoint(value: &str) -> Result<Url> {
    let url = Url::parse(value.trim()).context("Некорректный URL провайдера")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Ожидается HTTP(S) URL без учётных данных"
    );
    Ok(url)
}

pub fn is_local(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

pub fn api(url: &Url, path: &str) -> Url {
    let mut url = url.clone();
    url.set_path(&format!("{}/{}", url.path().trim_end_matches('/'), path));
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_local_engines_and_selects_the_llama_server() {
        let folder = tempfile::tempdir().unwrap();
        let engine = folder.path().join("data/engines").join(if cfg!(windows) {
            "YaneuraOu.exe"
        } else {
            "YaneuraOu"
        });
        std::fs::create_dir_all(engine.parent().unwrap()).unwrap();
        std::fs::write(&engine, []).unwrap();
        let config = Config::from_lookup(folder.path().to_owned(), |key| match key {
            "REY_LLM_PROVIDER" => Some("llama_cpp".into()),
            "REY_LLAMA_CPP_HOST" => Some("http://127.0.0.1:9931/v1".into()),
            "REY_LLAMA_CPP_MODEL" => Some("test-model".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.provider, ProviderKind::LlamaCpp);
        assert_eq!(config.provider_url().port(), Some(9931));
        assert_eq!(config.model(), "test-model");
        assert_eq!(config.shogi_engine, engine);
    }

    #[test]
    fn old_database_stays_separate_and_remote_memory_defaults_off() {
        let config = Config::from_lookup(PathBuf::from("rey"), |key| match key {
            "DATABASE_URL" => Some("sqlite:///data/custom.sqlite3".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.legacy_database, Path::new("rey/data/custom.sqlite3"));
        assert_eq!(config.database, Path::new("rey/ghost/memory.db"));
        assert!(!config.remote_memory);
        assert!(!is_local(
            &endpoint("https://localhost.example.org").unwrap()
        ));
        assert!(is_local(&endpoint("http://[::1]:11434").unwrap()));
        assert!(endpoint("http://name:secret@localhost").is_err());
    }
}
