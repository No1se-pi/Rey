use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin, time::Duration};

use crate::{
    config::{Config, ProviderKind, api, is_local},
    memory::Message,
};

mod llama_cpp;

pub fn from_config(config: &Config) -> Result<Box<dyn Provider>> {
    match config.provider {
        ProviderKind::Ollama => Ok(Box::new(Ollama::new(
            config.ollama.clone(),
            config.timeout,
        )?)),
        ProviderKind::LlamaCpp => Ok(Box::new(llama_cpp::LlamaCpp::new(config)?)),
    }
}

type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
type OnText<'a> = dyn FnMut(&str) + Send + 'a;
const GAME_REPLY_TOKENS: u16 = 144;

pub trait Provider: Send + Sync {
    fn chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [Value],
    ) -> Call<'a, ChatMessage>;
    fn game_chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        on_text: &'a mut OnText<'_>,
    ) -> Call<'a, ()>;
    fn models(&self) -> Call<'_, Vec<String>>;
    fn unload<'a>(&'a self, model: &'a str) -> Call<'a, ()>;
    fn is_local(&self) -> bool;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl From<Message> for ChatMessage {
    fn from(message: Message) -> Self {
        Self {
            role: message.role,
            content: message.content,
            tool_calls: Vec::new(),
            tool_name: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub function: Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub arguments: Value,
}

pub struct Ollama {
    base: Url,
    client: Client,
}

impl Ollama {
    pub fn new(base: Url, timeout: Duration) -> Result<Self> {
        Ok(Self {
            base,
            client: http_client(timeout)?,
        })
    }

    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[Value],
    ) -> Result<ChatMessage> {
        #[derive(Deserialize)]
        struct ChatResponse {
            message: ChatMessage,
        }
        let mut payload = json!({"model": model, "messages": messages, "stream": false});
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }
        let response = self
            .client
            .post(api(&self.base, "api/chat"))
            .json(&payload)
            .send()
            .await
            .context("Ollama недоступна. Проверь адрес и запусти ollama serve")?;
        if response.status().is_client_error() && !tools.is_empty() {
            let status = response.status();
            let body = response.text().await?;
            if status.as_u16() == 400 && body.to_lowercase().contains("does not support tools") {
                return Box::pin(self.generate(model, messages, &[])).await;
            }
            bail!("Ollama вернула HTTP {status}. Проверь выбранную модель");
        }
        let response: ChatResponse = response
            .error_for_status()?
            .json()
            .await
            .context("Некорректный ответ Ollama")?;
        ensure!(
            response.message.role == "assistant",
            "Ollama вернула неверную роль сообщения"
        );
        Ok(response.message)
    }
}

pub(crate) fn http_client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}

pub(super) enum StreamLine {
    Skip,
    Text(String),
    Last(String),
}

pub(super) async fn read_chat_stream(
    mut response: reqwest::Response,
    mut decode: impl FnMut(&[u8]) -> Result<StreamLine>,
    on_text: &mut OnText<'_>,
) -> Result<()> {
    let mut pending = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        for part in chunk.split_inclusive(|&byte| byte == b'\n') {
            ensure!(
                pending.len() + part.len() <= 256 * 1024,
                "Слишком длинная строка в ответе модели"
            );
            pending.extend_from_slice(part);
            if part.ends_with(b"\n") {
                let line = decode(&pending)?;
                pending.clear();
                match line {
                    StreamLine::Skip => {}
                    StreamLine::Text(text) => on_text(&text),
                    StreamLine::Last(text) => {
                        on_text(&text);
                        return Ok(());
                    }
                }
            }
        }
    }
    // A server may omit the final newline.
    if !pending.is_empty() {
        match decode(&pending)? {
            StreamLine::Last(text) => {
                on_text(&text);
                return Ok(());
            }
            StreamLine::Text(text) => on_text(&text),
            StreamLine::Skip => {}
        }
    }
    bail!("Соединение с моделью закрылось до конца ответа")
}

impl Provider for Ollama {
    fn chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [Value],
    ) -> Call<'a, ChatMessage> {
        Box::pin(self.generate(model, messages, tools))
    }

    fn game_chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        on_text: &'a mut OnText<'_>,
    ) -> Call<'a, ()> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Chunk {
                message: Option<ChatMessage>,
                #[serde(default)]
                done: bool,
                error: Option<String>,
            }
            let response = self
                .client
                .post(api(&self.base, "api/chat"))
                .json(&json!({
                    "model": model, "messages": messages, "stream": true, "think": false,
                    "options": {"num_predict": GAME_REPLY_TOKENS, "temperature": 0.3}
                }))
                .send()
                .await?
                .error_for_status()?;
            read_chat_stream(
                response,
                |line| {
                    if line.iter().all(u8::is_ascii_whitespace) {
                        return Ok(StreamLine::Skip);
                    }
                    let chunk: Chunk = serde_json::from_slice(line)?;
                    if let Some(error) = chunk.error {
                        bail!("{error}");
                    }
                    let text = chunk.message.map_or_else(String::new, |msg| msg.content);
                    if chunk.done {
                        // Ollama can attach its last fragment to the completion event.
                        return Ok(StreamLine::Last(text));
                    }
                    Ok(StreamLine::Text(text))
                },
                on_text,
            )
            .await
        })
    }

    fn models(&self) -> Call<'_, Vec<String>> {
        Box::pin(async {
            #[derive(Deserialize)]
            struct Model {
                name: String,
            }
            #[derive(Deserialize)]
            struct Models {
                models: Vec<Model>,
            }
            let result = self
                .client
                .get(api(&self.base, "api/tags"))
                .send()
                .await?
                .error_for_status()?
                .json::<Models>()
                .await?;
            let mut names: Vec<_> = result.models.into_iter().map(|model| model.name).collect();
            names.sort();
            Ok(names)
        })
    }

    fn unload<'a>(&'a self, model: &'a str) -> Call<'a, ()> {
        Box::pin(async move {
            if self.is_local() {
                self.client
                    .post(api(&self.base, "api/generate"))
                    .timeout(Duration::from_secs(15))
                    .json(&json!({"model":model,"keep_alive":0,"stream":false}))
                    .send()
                    .await?
                    .error_for_status()?;
            }
            Ok(())
        })
    }

    fn is_local(&self) -> bool {
        is_local(&self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    #[tokio::test]
    async fn game_commentary_delivers_text_before_the_ollama_stream_ends() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let (shown, preview) = oneshot::channel();
        let handler = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).await.unwrap();
            assert!(String::from_utf8_lossy(&buffer[..size]).starts_with("POST /api/chat"));
            let first =
                "{\"message\":{\"role\":\"assistant\",\"content\":\"Ход\"},\"done\":false}\n";
            let last =
                "{\"message\":{\"role\":\"assistant\",\"content\":\" e4.\"},\"done\":true}\n";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                first.len() + last.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            // Split a UTF-8 character across writes.
            let split = first.find('Х').unwrap() + 1;
            socket.write_all(&first.as_bytes()[..split]).await.unwrap();
            socket.write_all(&first.as_bytes()[split..]).await.unwrap();
            assert_eq!(preview.await.unwrap(), "Ход");
            socket.write_all(last.as_bytes()).await.unwrap();
        });
        let mut shown = Some(shown);
        let mut result = String::new();
        Ollama::new(url, Duration::from_secs(3))
            .unwrap()
            .game_chat("test", &[], &mut |fragment| {
                result.push_str(fragment);
                if let Some(shown) = shown.take() {
                    shown.send(fragment.to_string()).unwrap();
                }
            })
            .await
            .unwrap();
        assert_eq!(result, "Ход e4.");
        handler.await.unwrap();
    }

    #[tokio::test]
    async fn ollama_tags_are_read_over_http() {
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let handler = tokio::spawn(async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).await.unwrap();
            assert!(String::from_utf8_lossy(&buffer[..size]).starts_with("GET /api/tags"));
            let body = r#"{"models":[{"name":"test:latest"}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        assert_eq!(
            Ollama::new(url, Duration::from_secs(2))
                .unwrap()
                .models()
                .await
                .unwrap(),
            ["test:latest"]
        );
        handler.await.unwrap();
    }
}
