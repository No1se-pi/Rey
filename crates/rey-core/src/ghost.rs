use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[derive(Clone, Debug, Deserialize)]
pub struct Soul {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub principles: Vec<String>,
    pub system_prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Character {
    pub version: u32,
    pub manner: String,
    pub interests: Vec<String>,
    pub habits: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Ghost {
    pub soul: Soul,
    pub character: Character,
    pub digest: String,
}

impl Ghost {
    pub fn load(root: &Path) -> Result<Self> {
        let bytes = fs::read(root.join("ghost/soul.json")).context("Не найден ghost/soul.json")?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if let Some(expected) = std::env::var_os("REY_SOUL_SHA256") {
            ensure!(
                expected
                    .to_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&digest)),
                "Контрольная сумма soul.json не совпадает с REY_SOUL_SHA256"
            );
        }
        Ok(Self {
            soul: serde_json::from_slice(&bytes).context("Некорректный soul.json")?,
            character: serde_json::from_slice(
                &fs::read(root.join("ghost/character.json"))
                    .context("Не найден ghost/character.json")?,
            )
            .context("Некорректный character.json")?,
            digest,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chapter {
    Index,
    Memory,
    Skills,
    Scripting,
    Environment,
    Limitations,
}

impl Chapter {
    pub fn text(self) -> &'static str {
        match self {
            Self::Index => include_str!("../../../almanac/index.md"),
            Self::Memory => include_str!("../../../almanac/memory.md"),
            Self::Skills => include_str!("../../../almanac/skills.md"),
            Self::Scripting => include_str!("../../../almanac/scripting.md"),
            Self::Environment => include_str!("../../../almanac/environment.md"),
            Self::Limitations => include_str!("../../../almanac/limitations.md"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_ghost_uses_the_original_identity() {
        let ghost = Ghost::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .as_path(),
        )
        .unwrap();
        assert!(ghost.soul.system_prompt.contains("Ты - Рей"));
        assert_eq!(ghost.digest.len(), 64);
        assert!(Chapter::Memory.text().contains("memory_search"));
    }
}
