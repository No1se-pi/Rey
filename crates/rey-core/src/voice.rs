use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};

use crate::{
    config::{Config, api},
    llm::http_client,
};

#[derive(Clone)]
pub struct Voice {
    config: Config,
    lock: Arc<Mutex<()>>,
}

pub struct Audio {
    folder: TempDir,
}

impl Audio {
    pub fn wav(&self) -> PathBuf {
        self.folder.path().join("reply.wav")
    }
    pub fn archive(&self, directory: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(directory)?;
        let output = tempfile::Builder::new()
            .prefix("rey-")
            .tempdir_in(directory)?;
        for file in ["reply.wav", "reply.mp3", "reply.ogg"] {
            std::fs::copy(self.folder.path().join(file), output.path().join(file))?;
        }
        Ok(output.keep())
    }
}

#[derive(Deserialize)]
struct Transcript {
    text: String,
}

impl Voice {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn transcribe(&self, path: &Path) -> Result<String> {
        let _access = self.lock.lock().await;
        let metadata = tokio::fs::metadata(path)
            .await
            .context("Аудиофайл недоступен")?;
        ensure!(
            metadata.is_file()
                && metadata.len() > 0
                && metadata.len() <= self.config.max_audio_bytes,
            "Аудиофайл пуст или превышает лимит размера"
        );
        let transcript = if let Some(base) = &self.config.voice_url {
            let body = tokio::fs::read(path).await?;
            let response = http_client(Duration::from_secs(600))?
                .post(api(base, "v1/transcribe"))
                .header("Content-Type", "application/octet-stream")
                .body(body);
            let response = match &self.config.voice_token {
                Some(token) => response.bearer_auth(token),
                None => response,
            };
            let response = response.send().await?.error_for_status()?;
            serde_json::from_slice(&bounded_response(response, 32 * 1024).await?)?
        } else {
            let response = self
                .local("transcribe", &path.canonicalize()?, None)
                .await?;
            serde_json::from_slice::<Transcript>(&response)
                .context("Worker вернул некорректную транскрипцию")?
        };
        let text = transcript.text.trim();
        ensure!(
            !text.is_empty() && text.chars().count() <= 4000,
            "Транскрипция пуста или превышает 4000 символов"
        );
        Ok(text.to_owned())
    }

    pub async fn synthesize(&self, text: &str) -> Result<Audio> {
        let _access = self.lock.lock().await;
        let text: String = text.chars().take(4000).collect();
        ensure!(!text.trim().is_empty(), "Нет текста для озвучки");
        let folder = tempfile::tempdir()?;
        if let Some(base) = &self.config.voice_url {
            #[derive(Deserialize)]
            struct Encoded {
                wav: String,
                mp3: String,
                opus: String,
            }
            let response = http_client(Duration::from_secs(600))?
                .post(api(base, "v1/synthesize"))
                .json(&json!({"text":text}));
            let response = match &self.config.voice_token {
                Some(token) => response.bearer_auth(token),
                None => response,
            };
            let response = response.send().await?.error_for_status()?;
            let encoded: Encoded =
                serde_json::from_slice(&bounded_response(response, 64 * 1024 * 1024).await?)?;
            for (filename, encoded) in [
                ("reply.wav", encoded.wav),
                ("reply.mp3", encoded.mp3),
                ("reply.ogg", encoded.opus),
            ] {
                tokio::fs::write(folder.path().join(filename), STANDARD.decode(encoded)?).await?;
            }
        } else {
            self.local(
                "synthesize",
                folder.path(),
                Some(serde_json::to_vec(&json!({"text":text}))?),
            )
            .await?;
        }
        for filename in ["reply.wav", "reply.mp3", "reply.ogg"] {
            let metadata = tokio::fs::metadata(folder.path().join(filename))
                .await
                .context("Worker не вернул все аудиофайлы")?;
            ensure!(
                metadata.len() > 0 && metadata.len() <= 48 * 1024 * 1024,
                "Некорректный размер ответа TTS"
            );
        }
        Ok(Audio { folder })
    }

    async fn local(
        &self,
        operation: &str,
        path: &Path,
        payload: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.config.python);
        let binaries = self.config.root.join("data/bin");
        if binaries.is_dir() {
            let inherited = std::env::var_os("PATH").unwrap_or_default();
            let entries = std::iter::once(binaries).chain(std::env::split_paths(&inherited));
            command.env("PATH", std::env::join_paths(entries)?);
        }
        command
            .current_dir(&self.config.root)
            .args(["-m", "providers.python_inference", operation])
            .arg(path)
            .env("PYTHONUTF8", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .stdin(if payload.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .context("Не удалось запустить Python worker. Проверь REY_PYTHON")?;
        let output = tokio::time::timeout(Duration::from_secs(600), async move {
            if let Some(payload) = payload {
                let mut stdin = child.stdin.take().context("Worker не открыл stdin")?;
                stdin.write_all(&payload).await?;
            }
            let output = child.wait_with_output().await?;
            ensure!(
                output.status.success(),
                "Голосовой worker завершился с ошибкой. Проверь его stderr и установку моделей"
            );
            ensure!(
                output.stdout.len() <= 32 * 1024,
                "Слишком большой ответ worker"
            );
            Ok(output.stdout)
        })
        .await
        .context("Истёк таймаут голосового worker")??;
        Ok(output)
    }
}

async fn bounded_response(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            data.len() + chunk.len() <= max,
            "Слишком большой ответ голосового провайдера"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_missing_and_empty_audio_before_inference() {
        let root = tempfile::tempdir().unwrap();
        let voice = Voice::new(Config::load(root.path()).unwrap());
        let path = root.path().join("input.wav");
        assert!(voice.transcribe(&path).await.is_err());
        tokio::fs::write(&path, b"").await.unwrap();
        assert!(
            voice
                .transcribe(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("пуст")
        );
        assert!(voice.synthesize("  ").await.is_err());
    }
}
