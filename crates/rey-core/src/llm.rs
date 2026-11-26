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

pub trait Provider: Send + Sync {
    fn chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [Value],
    ) -> Call<'a, ChatMessage>;
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

impl Provider for Ollama {
    fn chat<'a>(
        &'a self,
        model: &'a str,
        messages: &'a [ChatMessage],
        tools: &'a [Value],
    ) -> Call<'a, ChatMessage> {
        Box::pin(self.generate(model, messages, tools))
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
    };

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
