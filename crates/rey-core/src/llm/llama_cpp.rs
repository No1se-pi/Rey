use super::{
    Call, ChatMessage, Function, GAME_REPLY_TOKENS, OnText, Provider, StreamLine, ToolCall,
    http_client, read_chat_stream,
};
use crate::config::{Config, api, is_local};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, RequestBuilder, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) struct LlamaCpp {
    base: Url,
    client: Client,
    key: Option<String>,
}

impl LlamaCpp {
    pub(super) fn new(config: &Config) -> Result<Self> {
        let mut base = config.llama_cpp.clone();
        let path = base.path().trim_end_matches('/');
        if let Some(root) = path.strip_suffix("/v1") {
            let root = root.to_owned();
            base.set_path(&root);
        }
        if config.llama_cpp_key.is_some() || config.llama_cpp_key_file.is_some() {
            ensure!(
                is_local(&base) || base.scheme() == "https",
                "Для удалённого llama.cpp с ключом требуется HTTPS"
            );
        }
        let key = match (&config.llama_cpp_key, &config.llama_cpp_key_file) {
            (Some(key), _) => Some(key.trim().to_owned()),
            (None, Some(file)) => {
                let keys = std::fs::read_to_string(file)
                    .context("Не удалось прочитать файл ключа llama.cpp")?;
                Some(
                    keys.lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .context("Файл ключа llama.cpp пуст")?
                        .to_owned(),
                )
            }
            _ => None,
        };
        Ok(Self {
            base,
            client: http_client(config.timeout)?,
            key,
        })
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.key {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    async fn generate(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: &[Value],
    ) -> Result<ChatMessage> {
        let encoded: Vec<_> = messages.iter().map(encode_message).collect();
        let mut payload = json!({
            "model":model,"messages":encoded,"stream":false,"max_tokens":768,
            "chat_template_kwargs":{"enable_thinking":false},"cache_prompt":true
        });
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
        }
        let mut response = self
            .auth(self.client.post(api(&self.base, "v1/chat/completions")))
            .json(&payload)
            .send()
            .await
            .context("Нет соединения с llama.cpp. Проверь адрес llama-server")?;
        if response.status() == StatusCode::BAD_REQUEST && !tools.is_empty() {
            let error = response.text().await?.to_lowercase();
            if [
                "--jinja",
                "does not support tools",
                "tool use is not supported",
                "tool calling is not supported",
            ]
            .iter()
            .any(|phrase| error.contains(phrase))
            {
                payload
                    .as_object_mut()
                    .expect("payload is an object")
                    .remove("tools");
                response = self
                    .auth(self.client.post(api(&self.base, "v1/chat/completions")))
                    .json(&payload)
                    .send()
                    .await
                    .context("Нет соединения с llama.cpp")?;
            } else {
                bail!("llama.cpp отклонил запрос. Проверь модель и журнал llama-server");
            }
        }
        let response: Completion = checked_status(response)?
            .json()
            .await
            .context("Некорректный ответ llama.cpp")?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .context("llama.cpp вернул ответ без вариантов")?;
        ensure!(
            choice.message.role == "assistant",
            "llama.cpp вернул неверную роль сообщения"
        );
        let calls = choice
            .message
            .tool_calls
            .into_iter()
            .map(|call| {
                let raw = call.function.arguments;
                let arguments = match raw {
                    Value::String(ref value) => serde_json::from_str(value).unwrap_or(raw),
                    value => value,
                };
                ToolCall {
                    id: Some(call.id),
                    function: Function {
                        name: call.function.name,
                        arguments,
                    },
                }
            })
            .collect();
        Ok(ChatMessage {
            role: choice.message.role,
            content: choice.message.content.unwrap_or_default(),
            tool_calls: calls,
            tool_name: None,
            tool_call_id: None,
        })
    }
}

fn checked_status(response: reqwest::Response) -> Result<reqwest::Response> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            bail!("llama.cpp отклонил ключ. Проверь REY_LLAMA_CPP_API_KEY_FILE")
        }
        _ => Ok(response.error_for_status()?),
    }
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: CompletionMessage,
}

#[derive(Deserialize)]
struct CompletionMessage {
    role: String,
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ReturnedToolCall>,
}

#[derive(Deserialize)]
struct ReturnedToolCall {
    id: String,
    function: Function,
}

fn encode_message(message: &ChatMessage) -> Value {
    let mut value = json!({"role":message.role,"content":message.content});
    if let Some(id) = &message.tool_call_id {
        value["tool_call_id"] = json!(id);
    }
    if let Some(name) = &message.tool_name {
        value["name"] = json!(name);
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] =
            json!(message.tool_calls.iter().map(|call| json!({
            "id":call.id,"type":"function","function":{
                "name":call.function.name,"arguments":call.function.arguments.to_string()
            }
        })).collect::<Vec<_>>());
    }
    value
}

impl Provider for LlamaCpp {
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
            struct Delta {
                content: Option<String>,
            }
            #[derive(Deserialize)]
            struct StreamChoice {
                delta: Delta,
            }
            #[derive(Deserialize)]
            struct Chunk {
                choices: Vec<StreamChoice>,
            }
            let encoded: Vec<_> = messages.iter().map(encode_message).collect();
            let response = self
                .auth(self.client.post(api(&self.base, "v1/chat/completions")))
                .json(&json!({
                    "model": model, "messages": encoded, "stream": true,
                    "max_tokens": GAME_REPLY_TOKENS, "temperature": 0.3,
                    "chat_template_kwargs": {"enable_thinking": false}, "cache_prompt": true
                }))
                .send()
                .await?;
            read_chat_stream(
                checked_status(response)?,
                |line| {
                    let line = std::str::from_utf8(line)?.trim_end();
                    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                        return Ok(StreamLine::Skip);
                    };
                    if data == "[DONE]" {
                        return Ok(StreamLine::Last(String::new()));
                    }
                    let chunk: Chunk = serde_json::from_str(data)?;
                    Ok(chunk
                        .choices
                        .into_iter()
                        .next()
                        .and_then(|choice| choice.delta.content)
                        .map_or(StreamLine::Skip, StreamLine::Text))
                },
                on_text,
            )
            .await
        })
    }

    fn models(&self) -> Call<'_, Vec<String>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Models {
                data: Vec<Model>,
            }
            #[derive(Deserialize)]
            struct Model {
                id: String,
            }
            let response = self
                .auth(self.client.get(api(&self.base, "v1/models")))
                .send()
                .await
                .context("Нет соединения с llama.cpp. Проверь адрес llama-server")?;
            let models: Models = checked_status(response)?
                .json()
                .await
                .context("Некорректный список моделей llama.cpp")?;
            Ok(models.data.into_iter().map(|model| model.id).collect())
        })
    }

    fn unload<'a>(&'a self, _model: &'a str) -> Call<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn is_local(&self) -> bool {
        is_local(&self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Message;

    #[test]
    fn encodes_openai_tool_round_trip() {
        let mut response: ChatMessage = Message::new("assistant", "").into();
        response.tool_calls.push(ToolCall {
            id: Some("call_1".into()),
            function: Function {
                name: "memory_read".into(),
                arguments: json!({"id":5}),
            },
        });
        let wire = encode_message(&response);
        assert_eq!(wire["tool_calls"][0]["type"], "function");
        assert_eq!(wire["tool_calls"][0]["function"]["arguments"], "{\"id\":5}");
        let reply = ChatMessage {
            tool_call_id: Some("call_1".into()),
            ..Message::new("tool", "result").into()
        };
        assert_eq!(encode_message(&reply)["tool_call_id"], "call_1");
    }
}
