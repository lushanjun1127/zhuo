//! OpenAI 兼容 `/chat/completions` 客户端 + 按 SSE 事件帧解析的流式切分。

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// 默认推理端点：本地 Ollama 的 OpenAI 兼容层。
/// 与 Ollama 的已知差异均已兼容：首帧 delta 带 role、独立 finish_reason 帧、
/// `choices:[]` + usage + timings 的用量帧（include_usage 会被 Ollama 接受）。
/// 可用 `ZHUO_API_BASE` 环境变量覆盖（如换成 llama-server 的 `http://127.0.0.1:8080/v1`）。
pub const DEFAULT_BASE: &str = "http://localhost:11434/v1";

/// 默认 API key：Ollama 不校验 key，占位即可。
pub const DEFAULT_API_KEY: &str = "ollama";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Msg {
    pub role: String,
    pub content: String,
}

impl Msg {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::new("user", content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new("assistant", content)
    }
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: &'a [Msg],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptionsReq>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormatReq<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repetition_penalty: Option<f32>,
    /// Ollama 扩展：GPU 层数（99 = 尽量全上 GPU，0 = 纯 CPU）。
    /// OpenAI 官方端点不认识该字段，仅在本地 Ollama 目标上发送。
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ChatOptions>,
}

#[derive(Serialize)]
struct ChatOptions {
    num_gpu: i64,
}

#[derive(Serialize)]
struct ResponseFormatReq<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Serialize)]
struct StreamOptionsReq {
    include_usage: bool,
}

/// Token 用量。不同厂商可能返回数字或字符串，统一收成 u64。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    Token(String),
    Usage(Usage),
    Done,
}

/// SSE 事件帧解析器。按**字节**缓冲，只在完整行处解码，避免中文被 TCP 分包切成半个字符。
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data_lines: Vec<String>,
    /// 是否收到 `data: [DONE]`（没收到不代表失败，EOF 正常收尾即可）
    pub done: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段原始字节，返回本次完整事件解析出的 SseEvent 列表
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>> {
        if self.done {
            return Ok(Vec::new());
        }
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(|c| c == '\r' || c == '\n');
            self.handle_line(line, &mut events)?;
            if self.done {
                break;
            }
        }
        Ok(events)
    }

    /// 流结束：冲刷末行与未以空行结尾的最后一个事件（兼容「无 [DONE]」的服务端）
    pub fn finish(&mut self) -> Result<Vec<SseEvent>> {
        let mut events = Vec::new();
        if self.done {
            return Ok(events);
        }
        if !self.buf.is_empty() {
            let raw = std::mem::take(&mut self.buf);
            let line = String::from_utf8_lossy(&raw).to_string();
            let line = line
                .trim_end_matches(|c| c == '\r' || c == '\n')
                .to_string();
            self.handle_line(&line, &mut events)?;
        }
        self.dispatch(&mut events)?;
        Ok(events)
    }

    fn handle_line(&mut self, line: &str, events: &mut Vec<SseEvent>) -> Result<()> {
        if line.is_empty() {
            return self.dispatch(events);
        }
        if line.starts_with(':') {
            return Ok(()); // `: ping` 等注释/心跳
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => self.data_lines.push(value.to_string()),
            // event / id / retry 及未知字段按 SSE 规范忽略
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) -> Result<()> {
        if self.data_lines.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.data_lines).join("\n");
        parse_event(&data, events, &mut self.done)
    }
}

fn parse_event(data: &str, events: &mut Vec<SseEvent>, done: &mut bool) -> Result<()> {
    let data = data.trim();
    if data.is_empty() || *done {
        return Ok(());
    }
    if data == "[DONE]" {
        *done = true;
        events.push(SseEvent::Done);
        return Ok(());
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // 坏帧 / 非 JSON 心跳，忽略
    };
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or(data);
        return Err(anyhow!("SSE error frame: {msg}"));
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        events.push(SseEvent::Usage(Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(num),
            completion_tokens: u.get("completion_tokens").and_then(num),
            total_tokens: u.get("total_tokens").and_then(num),
        }));
    }
    if let Some(content) = v
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
    {
        if !content.is_empty() {
            events.push(SseEvent::Token(content.to_string()));
        }
    }
    Ok(())
}

fn num(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[derive(Debug, Clone)]
pub struct StreamConfig<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    /// 请求带 stream_options.include_usage，用于取真 API 的 prompt_tokens 基线
    pub include_usage: bool,
    /// 原始 SSE 字节流落盘路径（None 不 dump）
    pub dump_path: Option<&'a Path>,
    /// 强制 JSON 输出模式（如 "json_object"），用于让模型稳定吐合法 JSON（抽取类任务）
    pub response_format: Option<&'a str>,
    /// 采样温度（None 用服务端默认）；抽取类任务建议设 0 以获得稳定可复现结果
    pub temperature: Option<f32>,
    /// Maximum generated tokens. None uses the provider default.
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repetition_penalty: Option<f32>,
    /// Ollama GPU/CPU 分流：None = 不发 options；Some(99) 全 GPU，Some(0) 强制 CPU。
    pub num_gpu: Option<i64>,
    /// true = 走 Ollama 原生 `/api/chat`，让 options.num_gpu 真正生效。
    /// Ollama 的 OpenAI 兼容层会丢弃请求体里的 options 字段，不可用。
    pub ollama_native: bool,
}

#[derive(Debug, Clone, Default)]
pub struct StreamOutcome {
    pub content: String,
    pub usage: Option<Usage>,
    /// 是否收到 [DONE]；false 表示服务端直接断流，内容仍可能完整
    pub done: bool,
    /// 是否被调用方主动中断（on_token 返回 false）；部分内容保留在 content
    pub aborted: bool,
}

/// 应用一个事件到 outcome。返回 false = 调用方要求中断（不再拉取后续字节）。
fn apply(
    ev: SseEvent,
    outcome: &mut StreamOutcome,
    on_token: &mut impl FnMut(&str) -> bool,
) -> bool {
    match ev {
        SseEvent::Token(t) => {
            outcome.content.push_str(&t);
            on_token(&t)
        }
        SseEvent::Usage(u) => {
            outcome.usage = Some(u);
            true
        }
        SseEvent::Done => true,
    }
}

/// 流式对话（扩展版）：逐 token 回调 + 用量采集 + 可选原始字节 dump。
/// `on_token` 返回 `false` 表示**主动中断**：立刻停止拉取上游字节（等价断开对
/// 推理服务的连接），已收到的部分内容保留在 `StreamOutcome::content`，
/// `aborted = true`。典型用途：用户点「停止生成」/ 客户端断开时释放显存。
/// 本地 Ollama 的 OpenAI 兼容层（`middleware/openai.go`）会把请求体重写成
/// `api.ChatRequest`，`ChatCompletionRequest` 并没有 `options` 字段，客户端塞的
/// `options.num_gpu` 会被静默丢弃。要让 GPU/CPU 分流真正生效，必须走原生 `/api/chat`。
fn ollama_native_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let root = base
        .strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/');
    format!("{root}/api/chat")
}

/// 原生 `/api/chat` 请求体：options.num_gpu 在这里才是 Ollama 认的参数。
fn ollama_native_body(cfg: &StreamConfig<'_>, messages: &[Msg], num_gpu: i64) -> Value {
    let mut options = serde_json::Map::new();
    options.insert("num_gpu".into(), json!(num_gpu));
    if let Some(v) = cfg.temperature {
        options.insert("temperature".into(), json!(v));
    }
    if let Some(v) = cfg.max_tokens {
        options.insert("num_predict".into(), json!(v));
    }
    if let Some(v) = cfg.top_p {
        options.insert("top_p".into(), json!(v));
    }
    if let Some(v) = cfg.frequency_penalty {
        options.insert("frequency_penalty".into(), json!(v));
    }
    if let Some(v) = cfg.presence_penalty {
        options.insert("presence_penalty".into(), json!(v));
    }
    if let Some(v) = cfg.repetition_penalty {
        options.insert("repeat_penalty".into(), json!(v));
    }
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "stream": true,
        "options": options,
    });
    if cfg.response_format == Some("json_object") {
        body["format"] = json!("json");
    }
    body
}

/// 把 Ollama 原生 NDJSON 帧转换成 OpenAI SSE 字节，复用 SseParser/usage/abort 逻辑。
/// 按字节缓冲到 `\n` 再解码，中文被 TCP 分包也不会切出半个 UTF-8 字符。
#[derive(Default)]
struct NdjsonBridge {
    pending: Vec<u8>,
}

impl NdjsonBridge {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<u8>> {
        self.pending.extend_from_slice(chunk);
        self.drain_lines()
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        if !self.pending.is_empty() {
            self.pending.push(b'\n');
        }
        self.drain_lines()
    }

    fn drain_lines(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                continue; // 坏帧忽略，与 SSE 解析器一致
            };
            if let Some(err) = v.get("error").and_then(Value::as_str) {
                return Err(anyhow!("Ollama error frame: {err}"));
            }
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_str) {
                if !content.is_empty() {
                    out.extend_from_slice(
                        format!(
                            "data: {}\r\n\r\n",
                            json!({ "choices": [{ "delta": { "content": content } }] })
                        )
                        .as_bytes(),
                    );
                }
            }
            if v.get("done").and_then(Value::as_bool) == Some(true) {
                let prompt = v.get("prompt_eval_count").and_then(Value::as_u64);
                let completion = v.get("eval_count").and_then(Value::as_u64);
                let total = match (prompt, completion) {
                    (Some(p), Some(c)) => Some(p + c),
                    _ => None,
                };
                out.extend_from_slice(
                    format!(
                        "data: {}\r\n\r\n",
                        json!({ "choices": [], "usage": {
                            "prompt_tokens": prompt,
                            "completion_tokens": completion,
                            "total_tokens": total,
                        }})
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(b"data: [DONE]\r\n\r\n");
            }
        }
        Ok(out)
    }
}

pub async fn stream_chat_ex(
    cfg: &StreamConfig<'_>,
    messages: &[Msg],
    mut on_token: impl FnMut(&str) -> bool,
) -> Result<StreamOutcome> {
    // 带 num_gpu 且开启原生通道 → Ollama 原生端点，options 才会真正生效；
    // 否则退化为标准 OpenAI-compatible 请求（options 仍会带上，供兼容端点读取）。
    let native = cfg.ollama_native && cfg.num_gpu.is_some();
    let url = if native {
        ollama_native_url(cfg.base_url)
    } else {
        format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'))
    };
    let body = if native {
        ollama_native_body(cfg, messages, cfg.num_gpu.unwrap())
    } else {
        serde_json::to_value(ChatReq {
            model: cfg.model,
            messages,
            stream: true,
            stream_options: cfg.include_usage.then_some(StreamOptionsReq {
                include_usage: true,
            }),
            response_format: cfg.response_format.map(|k| ResponseFormatReq { kind: k }),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            top_p: cfg.top_p,
            frequency_penalty: cfg.frequency_penalty,
            presence_penalty: cfg.presence_penalty,
            repetition_penalty: cfg.repetition_penalty,
            options: cfg.num_gpu.map(|n| ChatOptions { num_gpu: n }),
        })?
    };
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .bearer_auth(cfg.api_key)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.context("read upstream error response")?;
        let detail: String = body.trim().chars().take(2048).collect();
        let truncated = body.trim().chars().count() > 2048;
        let detail = if truncated {
            format!("{detail}... (response truncated)")
        } else {
            detail
        };
        if detail.is_empty() {
            return Err(anyhow!("upstream returned HTTP {status}"));
        }
        return Err(anyhow!("upstream returned HTTP {status}: {detail}"));
    }

    let mut dump: Option<std::fs::File> = match cfg.dump_path {
        Some(p) => {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Some(
                std::fs::File::create(p)
                    .with_context(|| format!("create SSE dump: {}", p.display()))?,
            )
        }
        None => None,
    };

    let mut parser = SseParser::new();
    let mut bridge = NdjsonBridge::default();
    let mut outcome = StreamOutcome::default();
    let mut aborted = false;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(f) = dump.as_mut() {
            f.write_all(&chunk)?;
        }
        let framed = if native {
            bridge.push(&chunk)?
        } else {
            chunk.to_vec()
        };
        for ev in parser.push(&framed)? {
            if !apply(ev, &mut outcome, &mut on_token) {
                aborted = true;
                break;
            }
        }
        if aborted || parser.done {
            break;
        }
    }
    if !aborted {
        if native {
            for ev in parser.push(&bridge.finish()?)? {
                if !apply(ev, &mut outcome, &mut on_token) {
                    aborted = true;
                    break;
                }
            }
        }
        if !aborted {
            for ev in parser.finish()? {
                apply(ev, &mut outcome, &mut on_token);
            }
        }
    }
    if let Some(f) = dump.as_mut() {
        f.flush()?;
    }
    outcome.done = parser.done && !aborted;
    outcome.aborted = aborted;
    Ok(outcome)
}

/// 兼容旧签名的最小封装
pub async fn stream_chat(
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[Msg],
    on_token: impl FnMut(&str) -> bool,
) -> Result<()> {
    let cfg = StreamConfig {
        base_url,
        api_key,
        model,
        include_usage: false,
        dump_path: None,
        response_format: None,
        temperature: None,
        max_tokens: None,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        repetition_penalty: None,
        num_gpu: None,
        ollama_native: false,
    };
    stream_chat_ex(&cfg, messages, on_token).await.map(|_| ())
}

/// 轻量 token 估算（**启发式，非真实分词器**）：
/// - CJK / 全角字符按 1 token/字（cl100k 对常见汉字多为单 token）
/// - 其余非空白字符按 ~4 字符/token 折算
/// 用途：DRY_RUN 自检 prompt 体积、远端 key 到位前先估基线。
/// 真值仍以 StreamOutcome.usage 为准（远端支持 include_usage 时）。
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for c in text.chars() {
        if is_cjk(c) {
            cjk += 1;
        } else if !c.is_whitespace() {
            other += 1;
        }
    }
    cjk + (other + 3) / 4
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
        || ('\u{3400}'..='\u{4dbf}').contains(&c)
        || ('\u{20000}'..='\u{2a6df}').contains(&c)
        || ('\u{3000}'..='\u{303f}').contains(&c)
        || ('\u{ff00}'..='\u{ffef}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sse(data: &[&str]) -> Vec<u8> {
        let mut body = String::new();
        for d in data {
            body.push_str("data: ");
            body.push_str(d);
            body.push_str("\r\n\r\n");
        }
        body.push_str("data: [DONE]\r\n\r\n");
        body.into_bytes()
    }

    fn delta(s: &str) -> String {
        serde_json::json!({"choices":[{"delta":{"content":s}}]}).to_string()
    }

    fn tokens(events: &[SseEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                SseEvent::Token(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn parser_handles_split_lines_and_utf8() {
        let body = sse(&[&delta("你"), &delta("好"), &delta("，世界")]);
        let mut parser = SseParser::new();
        let mut out = String::new();
        // 逐字节喂入：模拟最极端的 TCP 分包（含中文多字节被切开）
        for b in &body {
            out.push_str(&tokens(&parser.push(&[*b]).unwrap()));
        }
        assert_eq!(out, "你好，世界");
        assert!(parser.done);
    }

    #[test]
    fn parser_handles_chunk_boundaries() {
        let body = sse(&[&delta("A"), &delta("B")]);
        let mid = body.len() / 2;
        let mut parser = SseParser::new();
        let mut out = String::new();
        for part in [&body[..mid], &body[mid..]] {
            out.push_str(&tokens(&parser.push(part).unwrap()));
        }
        assert_eq!(out, "AB");
    }

    #[test]
    fn parser_ignores_non_data_and_broken_json() {
        let body = b": ping\r\n\r\nevent: message\r\ndata: {broken}\r\n\r\ndata: {\"choices\":[{\"delta\":{}}]}\r\n\r\ndata: [DONE]\r\n\r\n";
        let mut parser = SseParser::new();
        let events = parser.push(body).unwrap();
        assert!(tokens(&events).is_empty());
        assert!(parser.done);
    }

    #[test]
    fn parser_accepts_no_space_after_data() {
        let mut parser = SseParser::new();
        let events = parser
            .push(format!("data:{}\n\n", delta("X")).as_bytes())
            .unwrap();
        assert_eq!(tokens(&events), "X");
    }

    #[test]
    fn error_frame_is_an_error() {
        let body =
            b"data: {\"error\":{\"message\":\"rate limited\",\"type\":\"rate_limit\"}}\r\n\r\n";
        let mut parser = SseParser::new();
        let err = parser.push(body).unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[test]
    fn usage_frame_and_string_numbers() {
        let body = b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":\"128\",\"completion_tokens\":7,\"total_tokens\":135}}\r\n\r\ndata: [DONE]\r\n\r\n";
        let mut parser = SseParser::new();
        let events = parser.push(body).unwrap();
        assert!(matches!(
            events.first(),
            Some(SseEvent::Usage(Usage {
                prompt_tokens: Some(128),
                ..
            }))
        ));
        assert_eq!(events.last(), Some(&SseEvent::Done));
    }

    /// 读完整个请求（含 body）再回包，否则过早关闭连接会触发 RST
    async fn read_request(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                return buf;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break p + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
        let len = headers
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + len {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        buf
    }

    /// 回包前把完整请求（含 body）记下来，供断言 options.num_gpu 与请求路径。
    async fn capture_mock_server(
        body: Vec<u8>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = captured.clone();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let raw = read_request(&mut sock).await;
            *sink.lock().unwrap() = String::from_utf8_lossy(&raw).to_string();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body).await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                let mut sink = [0u8; 1024];
                loop {
                    match sock.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
        });
        (format!("http://{}", addr), captured)
    }

    fn request_json(captured: &std::sync::Arc<std::sync::Mutex<String>>) -> serde_json::Value {
        let raw = captured.lock().unwrap().clone();
        let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        serde_json::from_str(body).expect("request body is valid JSON")
    }

    async fn mock_server(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let mid = body.len() / 2;
            let _ = sock.write_all(&body[..mid]).await;
            let _ = sock.write_all(&body[mid..]).await;
            // 等客户端读完并关闭再收连接：直接 shutdown 会因未读数据触发 RST
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                let mut sink = [0u8; 1024];
                loop {
                    match sock.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
        });
        format!("http://{}", addr)
    }

    async fn mock_error_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(body.as_bytes()).await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                let mut sink = [0u8; 1024];
                loop {
                    match sock.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn upstream_http_error_includes_response_body() {
        let base = mock_error_server(r#"{"error":"CUDA out of memory"}"#).await;
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: false,
            dump_path: None,
            response_format: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: Some(99),
            ollama_native: true,
        };
        let err = stream_chat_ex(&cfg, &[Msg::user("hi")], |_| true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{err}");
        assert!(err.to_string().contains("CUDA out of memory"), "{err}");
    }

    #[tokio::test]
    async fn stream_chat_against_mock_sse() {
        let base = mock_server(sse(&[&delta("你"), &delta("好")])).await;
        let msgs = vec![Msg::system("sys"), Msg::user("hi")];
        let mut out = String::new();
        stream_chat(&base, "test-key", "mock-model", &msgs, |t| {
            out.push_str(t);
            true
        })
        .await
        .unwrap();
        assert_eq!(out, "你好");
    }

    #[tokio::test]
    async fn stream_chat_ex_collects_usage_and_dumps_bytes() {
        let mut body = format!("data: {}\r\n\r\n", delta("糖葫芦")).into_bytes();
        // usage 帧必须在 [DONE] 之前
        body.extend_from_slice(
            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":42,\"completion_tokens\":3,\"total_tokens\":45}}\r\n\r\n",
        );
        body.extend_from_slice(b"data: [DONE]\r\n\r\n");
        let base = mock_server(body.clone()).await;
        let mut path = std::env::temp_dir();
        path.push(format!("zhuo_dump_test_{}.sse", std::process::id()));
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: true,
            dump_path: Some(&path),
            response_format: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: None,
            ollama_native: false,
        };
        let msgs = vec![Msg::user("hi")];
        let out = stream_chat_ex(&cfg, &msgs, |_| true).await.unwrap();
        assert_eq!(out.content, "糖葫芦");
        assert_eq!(out.usage.unwrap().prompt_tokens, Some(42));
        assert_eq!(std::fs::read(&path).unwrap(), body);
        let _ = std::fs::remove_file(&path);
    }

    /// 慢速 mock：逐字节写入（每字节 yield），用于测试「中断发生在确定的事件边界」
    async fn mock_server_slow(body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            for b in &body {
                let _ = sock.write_all(std::slice::from_ref(b)).await;
                tokio::task::yield_now().await;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                let mut sink = [0u8; 1024];
                loop {
                    match sock.read(&mut sink).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            })
            .await;
        });
        format!("http://{}", addr)
    }

    /// 用户点「停止生成」：回调返回 false → 立刻断开上游、保留已收内容、不报错
    #[tokio::test]
    async fn abort_mid_stream_keeps_partial_content() {
        let body = sse(&[&delta("冬"), &delta("堡"), &delta("下"), &delta("雪")]);
        let base = mock_server_slow(body).await;
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: false,
            dump_path: None,
            response_format: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: None,
            ollama_native: false,
        };
        let msgs = vec![Msg::user("hi")];
        let mut seen = 0usize;
        let out = stream_chat_ex(&cfg, &msgs, |_| {
            seen += 1;
            seen < 2 // 第 2 个 token 时中断
        })
        .await
        .unwrap();
        assert_eq!(
            out.content, "冬堡",
            "中断前已投递的 token 全部保留（含触发中断的那个）"
        );
        assert_eq!(seen, 2, "回调被调用 2 次（第 2 次返回 false）");
        assert!(out.aborted, "aborted 标记为 true");
        assert!(!out.done, "中断的流不算正常收尾");
        assert!(out.usage.is_none(), "中断后不再收 usage");
    }

    #[test]
    fn estimate_tokens_mixes_cjk_and_latin() {
        // 纯中文：每字约 1 token
        assert_eq!(estimate_tokens("冬堡下雪"), 4);
        // 纯拉丁：约每 4 字符 1 token（向上取整）
        assert_eq!(estimate_tokens("hello world"), 3); // 11 非空白字符 -> (11+3)/4 = 3
                                                       // 混合
        let mixed = estimate_tokens("冬堡 hello");
        // 2 个 CJK + 5 个拉丁非空白 -> 2 + (5+3)/4 = 2+2 = 4
        assert_eq!(mixed, 4);
        // 空白不计
        assert_eq!(estimate_tokens("a b c"), 1); // 3 非空白 -> (3+3)/4=1
    }

    #[test]
    fn default_base_points_to_local_ollama() {
        assert_eq!(DEFAULT_BASE, "http://localhost:11434/v1");
        assert_eq!(DEFAULT_API_KEY, "ollama");
    }

    /// GPU/CPU 分流：原生 /api/chat 请求体必须带 options.num_gpu，且 NDJSON 中文流正确解析
    #[tokio::test]
    async fn native_channel_sends_num_gpu_and_parses_ndjson() {
        let body = concat!(
            "{\"model\":\"m\",\"message\":{\"role\":\"assistant\",\"content\":\"冬\"},\"done\":false}\n",
            "{\"model\":\"m\",\"message\":{\"role\":\"assistant\",\"content\":\"堡\"},\"done\":false}\n",
            "{\"model\":\"m\",\"done\":true,\"prompt_eval_count\":42,\"eval_count\":2}\n",
        )
        .as_bytes()
        .to_vec();
        let (base, captured) = capture_mock_server(body).await;
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: true,
            dump_path: None,
            response_format: None,
            temperature: Some(0.0),
            max_tokens: Some(64),
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: Some(0),
            ollama_native: true,
        };
        let out = stream_chat_ex(&cfg, &[Msg::user("hi")], |_| true)
            .await
            .unwrap();
        assert_eq!(out.content, "冬堡");
        assert!(out.done);
        assert_eq!(out.usage.as_ref().unwrap().prompt_tokens, Some(42));
        let raw = captured.lock().unwrap().clone();
        assert!(
            raw.starts_with("POST /api/chat "),
            "必须走 Ollama 原生端点: {raw:?}"
        );
        let req = request_json(&captured);
        assert_eq!(req["options"]["num_gpu"], 0, "提取模型强制纯 CPU");
        assert_eq!(req["options"]["num_predict"], 64);
        assert_eq!(req["stream"], true);
    }

    /// 原生通道按字节缓冲：中文被逐字节分包也不会切出半个 UTF-8 字符
    #[tokio::test]
    async fn native_channel_survives_byte_by_byte_utf8_split() {
        let body = concat!(
            "{\"message\":{\"content\":\"糖\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"葫芦\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":7,\"eval_count\":3}\n",
        )
        .as_bytes()
        .to_vec();
        let base = mock_server_slow(body).await;
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: false,
            dump_path: None,
            response_format: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: Some(99),
            ollama_native: true,
        };
        let out = stream_chat_ex(&cfg, &[Msg::user("hi")], |_| true)
            .await
            .unwrap();
        assert_eq!(out.content, "糖葫芦");
        assert!(out.done);
    }

    /// 关闭原生通道时，标准 /chat/completions 请求体同样带 options.num_gpu
    #[tokio::test]
    async fn compat_channel_also_carries_options_num_gpu() {
        let (base, captured) = capture_mock_server(sse(&[&delta("好")])).await;
        let cfg = StreamConfig {
            base_url: &base,
            api_key: "test-key",
            model: "mock-model",
            include_usage: false,
            dump_path: None,
            response_format: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            repetition_penalty: None,
            num_gpu: Some(99),
            ollama_native: false,
        };
        let out = stream_chat_ex(&cfg, &[Msg::user("hi")], |_| true)
            .await
            .unwrap();
        assert_eq!(out.content, "好");
        let req = request_json(&captured);
        assert_eq!(req["options"]["num_gpu"], 99, "主模型全量上 GPU");
    }
}
