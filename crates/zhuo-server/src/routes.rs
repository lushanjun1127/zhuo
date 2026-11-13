//! HTTP 路由：SSE 流式对话（含记忆注入）+ 历史 + 重置 + 卡列表 + 记忆卡片管理。

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, Sse};
use axum::Json;
use base64::Engine;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use zhuo_llm::{StreamConfig, StreamOutcome};
use zhuo_memory::MemoryHit;

use crate::prompt::{assemble_messages, system_prompt};
use crate::state::ServerState;

/// 统一 SSE 流类型：盒装动态流，保证所有返回路径是同一个具体类型。
type SseStream = std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub card: String,
    #[serde(default)]
    pub chat_id: String,
    pub message: String,
    #[serde(default)]
    pub display_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegexScript {
    #[serde(default, alias = "scriptName")]
    pub script_name: String,
    #[serde(default, alias = "findRegex")]
    pub find_regex: String,
    #[serde(default, alias = "replaceString")]
    pub replace_string: String,
    #[serde(default)]
    pub placement: Vec<i64>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, alias = "promptOnly")]
    pub prompt_only: bool,
}

#[derive(Deserialize)]
pub struct CreateChatRequest {
    pub card: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Deserialize)]
pub struct RenameChatRequest {
    pub title: String,
}

#[derive(Deserialize)]
pub struct ImportCardRequest {
    pub filename: String,
    pub data: String,
}

#[derive(Deserialize)]
pub struct ExportQuery {
    #[serde(default = "default_export_format")]
    pub format: String,
}
fn default_export_format() -> String {
    "json".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSettings {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default)]
    pub frequency_penalty: f32,
    #[serde(default)]
    pub presence_penalty: f32,
    #[serde(default = "default_repetition_penalty")]
    pub repetition_penalty: f32,
}

fn default_provider() -> String {
    "ollama".into()
}
fn default_temperature() -> f32 {
    0.8
}
fn default_max_tokens() -> u32 {
    1024
}
fn default_top_p() -> f32 {
    1.0
}
fn default_repetition_penalty() -> f32 {
    1.0
}

/// 连接设置的合并视图：以 .env 为基线（ServerState 已读入 ZHUO_API_BASE /
/// ZHUO_API_KEY / ZHUO_MODEL），再用数据库里用户在设置面板保存的值覆盖。
/// 优先级：.env 显式配置的 api_key 最高，避免「只读数据库」导致 .env 的 key
/// 明明生效、接口却显示未配置的不一致。
fn connection_settings(st: &ServerState) -> ConnectionSettings {
    let mut cfg = ConnectionSettings {
        provider: "ollama".into(),
        base_url: (*st.base_url).clone(),
        api_key: (*st.api_key).clone(),
        model: (*st.model).clone(),
        temperature: 0.8,
        max_tokens: 1024,
        top_p: 1.0,
        frequency_penalty: 0.0,
        presence_penalty: 0.0,
        repetition_penalty: 1.0,
    };
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    let saved = mem
        .get_setting("connection")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<ConnectionSettings>(&s).ok());
    if let Some(saved) = saved {
        if !saved.base_url.trim().is_empty() {
            cfg.base_url = saved.base_url;
        }
        if !saved.model.trim().is_empty() {
            cfg.model = saved.model;
        }
        if !st.env_api_key && !saved.api_key.trim().is_empty() && saved.api_key != "••••••••"
        {
            cfg.api_key = saved.api_key;
        }
        cfg.provider = saved.provider;
        cfg.temperature = saved.temperature;
        cfg.max_tokens = saved.max_tokens;
        cfg.top_p = saved.top_p;
        cfg.frequency_penalty = saved.frequency_penalty;
        cfg.presence_penalty = saved.presence_penalty;
        cfg.repetition_penalty = saved.repetition_penalty;
    }
    cfg
}

/// 标注实际生效的 api_key 来源，便于前端与 .env 对账。
fn api_key_source(st: &ServerState) -> &'static str {
    if st.env_api_key {
        return "env";
    }
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    let in_db = mem
        .get_setting("connection")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<ConnectionSettings>(&s).ok())
        .map(|c| !c.api_key.trim().is_empty() && c.api_key != "••••••••")
        .unwrap_or(false);
    if in_db {
        "settings"
    } else {
        "default"
    }
}

#[derive(Deserialize)]
pub struct ResetRequest {
    pub card: String,
    #[serde(default)]
    pub memories: bool,
}

#[derive(Deserialize)]
pub struct AddMemoryRequest {
    pub card: String,
    pub summary: String,
    #[serde(default)]
    pub keywords: String,
    #[serde(default)]
    pub source_round: i64,
}

#[derive(Deserialize)]
pub struct SummarizeRequest {
    pub card: String,
}

#[derive(Deserialize)]
pub struct SetStateRequest {
    pub card: String,
    pub state: serde_json::Value,
}

fn to_payload(hits: &[MemoryHit]) -> serde_json::Value {
    json!({
        "memory_hits": hits.iter().map(|h| json!({
            "summary": h.summary,
            "keywords": h.keywords,
            "source_round": h.source_round,
        })).collect::<Vec<_>>(),
    })
}

/// GET /api/cards —— 卡列表（id/名称/描述/首句/system）。id 是内容短 hash，前端选卡用它。
/// system 是该卡实际注入的完整 System Prompt（角色设定 + 世界设定 + [当前状态]），
/// 前端「角色卡」面板用它排查出戏问题。
pub async fn cards(State(st): State<ServerState>) -> Json<serde_json::Value> {
    let list = st.list_cards();
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    Json(json!(list
        .iter()
        .map(|c| {
            let mut system = system_prompt(c, &st.world);
            if let Some(state) = mem.get_state(&c.id).ok().flatten() {
                if let Some(sb) = crate::prompt::render_state(&state) {
                    system.push_str("\n\n");
                    system.push_str(&sb);
                }
            }
            json!({
                "id": c.id,
                "name": c.name,
                "description": c.description,
                "first_mes": c.first_mes,
                "mes_example": c.mes_example,
                "system": system,
            })
        })
        .collect::<Vec<_>>()))
}

pub async fn import_card(
    State(st): State<ServerState>,
    Json(req): Json<ImportCardRequest>,
) -> Json<serde_json::Value> {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.data.trim()) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error": format!("角色卡不是有效的 Base64：{e}")})),
    };
    let raw = match zhuo_card::parse_card_bytes(&bytes) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error": format!("无法解析角色卡：{e:#}")})),
    };
    let card = match zhuo_card::Card::from_json(raw) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error": format!("角色卡字段无效：{e:#}")})),
    };
    let is_png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let ext = if is_png { "png" } else { "json" };
    let path = st.cards_dir.join(format!("{}.{}", card.id, ext));
    if let Err(e) =
        std::fs::create_dir_all(&*st.cards_dir).and_then(|_| std::fs::write(&path, &bytes))
    {
        return Json(json!({"error": format!("保存角色卡失败：{e}")}));
    }
    Json(
        json!({"ok": true, "card": {"id": card.id, "name": card.name, "description": card.description, "first_mes": card.first_mes}, "filename": req.filename}),
    )
}

pub async fn export_card(
    State(st): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Json<serde_json::Value> {
    let card = match st.find_card(&id) {
        Ok(c) => c,
        Err(e) => return Json(json!({"error":format!("{e:#}")})),
    };
    let v2 = card.to_v2_json();
    if q.format.eq_ignore_ascii_case("png") {
        let Some(path) = st.card_source_path(&card.id) else {
            return Json(json!({"error":"找不到角色卡源文件"}));
        };
        let source = match std::fs::read(path) {
            Ok(v) => v,
            Err(e) => return Json(json!({"error":e.to_string()})),
        };
        if !source.starts_with(b"\x89PNG") {
            return Json(json!({"error":"该角色没有 PNG 源图，请导出 V2 JSON"}));
        }
        match zhuo_card::write_v2_png(&source, &v2) {
            Ok(bytes) => Json(
                json!({"filename":format!("{}.png",card.name),"mime":"image/png","data":base64::engine::general_purpose::STANDARD.encode(bytes)}),
            ),
            Err(e) => Json(json!({"error":format!("{e:#}")})),
        }
    } else {
        let bytes = serde_json::to_vec_pretty(&v2).unwrap_or_default();
        Json(
            json!({"filename":format!("{}.json",card.name),"mime":"application/json","data":base64::engine::general_purpose::STANDARD.encode(bytes)}),
        )
    }
}

pub async fn import_worldbook(
    State(st): State<ServerState>,
    Json(req): Json<ImportCardRequest>,
) -> Json<serde_json::Value> {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.data.trim()) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":e.to_string()})),
    };
    let book = match zhuo_card::parse_world_book_bytes(&bytes) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":format!("{e:#}")})),
    };
    let raw = serde_json::to_string(&book).unwrap_or_default();
    let count = book.entries.len();
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.set_setting("worldbook", &raw) {
        Ok(()) => Json(json!({"ok":true,"name":book.name,"entries":count})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

pub async fn import_preset(
    State(st): State<ServerState>,
    Json(req): Json<ImportCardRequest>,
) -> Json<serde_json::Value> {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.data.trim()) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":e.to_string()})),
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":format!("预设 JSON 无效：{e}")})),
    };
    let mut cfg = connection_settings(&st);
    if let Some(n) = v.get("temperature").and_then(Value::as_f64) {
        cfg.temperature = n as f32
    }
    if let Some(n) = v.get("top_p").and_then(Value::as_f64) {
        cfg.top_p = n as f32
    }
    if let Some(n) = v.get("frequency_penalty").and_then(Value::as_f64) {
        cfg.frequency_penalty = n as f32
    }
    if let Some(n) = v.get("presence_penalty").and_then(Value::as_f64) {
        cfg.presence_penalty = n as f32
    }
    if let Some(n) = v
        .get("repetition_penalty")
        .or_else(|| v.get("rep_pen"))
        .and_then(Value::as_f64)
    {
        cfg.repetition_penalty = n as f32
    }
    if let Some(n) = v
        .get("openai_max_tokens")
        .or_else(|| v.get("max_tokens"))
        .and_then(Value::as_u64)
    {
        cfg.max_tokens = n.clamp(1, 32768) as u32
    }
    if let Some(m) = v
        .get("openai_model")
        .or_else(|| v.get("custom_model"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        cfg.model = m.into()
    }
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.set_setting("connection", &serde_json::to_string(&cfg).unwrap()) {
        Ok(()) => Json(json!({"ok":true,"connection":cfg})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

pub async fn import_regex(
    State(st): State<ServerState>,
    Json(req): Json<ImportCardRequest>,
) -> Json<serde_json::Value> {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(req.data.trim()) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":e.to_string()})),
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error":format!("Regex JSON 无效：{e}")})),
    };
    let scripts: Vec<RegexScript> = if value.is_array() {
        serde_json::from_value(value).unwrap_or_default()
    } else {
        serde_json::from_value(value)
            .map(|v| vec![v])
            .unwrap_or_default()
    };
    if scripts.is_empty() {
        return Json(json!({"error":"未找到有效的 ST Regex 脚本"}));
    }
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.set_setting("regex_scripts", &serde_json::to_string(&scripts).unwrap()) {
        Ok(()) => Json(json!({"ok":true,"scripts":scripts})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

pub async fn compatibility(State(st): State<ServerState>) -> Json<serde_json::Value> {
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    let book = mem
        .get_setting("worldbook")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<zhuo_card::WorldBook>(&s).ok());
    let scripts = mem
        .get_setting("regex_scripts")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<RegexScript>>(&s).ok())
        .unwrap_or_default();
    Json(json!({"worldbook":book,"regex_scripts":scripts}))
}

/// POST /api/chat —— SSE 流式对话。
/// 帧序：{"memory_hits":[...]} → {"token":"..."}* → {"usage":{...}} → {"error":...?} → [DONE]
pub async fn chat(State(st): State<ServerState>, Json(req): Json<ChatRequest>) -> Sse<SseStream> {
    // 1. 卡 + 状态 + 历史 + 记忆检索（短临界区，不跨 await 持锁）
    //    存储键一律用卡 id（内容短 hash）——同名卡彻底隔离
    let (card_key, active_chat, msgs, mem_hits) = {
        let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
        let card = match st.find_card(&req.card) {
            Ok(c) => c,
            Err(e) => return sse_error(format!("{e:#}")),
        };
        let active_chat = if req.chat_id.is_empty() {
            None
        } else {
            mem.get_chat(&req.chat_id).ok().flatten()
        };
        let history = match active_chat.as_ref() {
            Some(c) => mem.chat_history(&c.id, 200),
            None => mem.history(&card.id, 200),
        };
        let history = match history {
            Ok(h) => h,
            Err(e) => return sse_error(format!("read history: {e:#}")),
        };
        let hits = match mem.search(&card.id, &req.message, 5) {
            Ok(h) => h,
            Err(e) => return sse_error(format!("memory search: {e:#}")),
        };
        let mem_block = match mem.memory_block(&card.id, &req.message, 600) {
            Ok(b) => b,
            Err(e) => return sse_error(format!("memory block: {e:#}")),
        };
        // 状态机注入：有状态记录时在 System 尾部追加 [当前状态]
        let mut system = system_prompt(&card, &st.world);
        if let Some(state) = mem.get_state(&card.id).ok().flatten() {
            if let Some(sb) = crate::prompt::render_state(&state) {
                system.push_str("\n\n");
                system.push_str(&sb);
            }
        }
        let scan = history
            .iter()
            .map(|m| m.content.clone())
            .chain(std::iter::once(req.message.clone()))
            .collect::<Vec<_>>();
        let mut world_entries = Vec::new();
        if let Some(book) = &card.character_book {
            world_entries.extend(zhuo_card::activated_entries(book, &scan));
        }
        if let Some(book) = mem
            .get_setting("worldbook")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<zhuo_card::WorldBook>(&s).ok())
        {
            world_entries.extend(zhuo_card::activated_entries(&book, &scan));
        }
        if !world_entries.is_empty() {
            system.push_str("\n\n[World Info]\n");
            system.push_str(
                &world_entries
                    .iter()
                    .map(|e| e.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
        }
        let mut msgs = assemble_messages(
            &system,
            mem_block.as_deref(),
            &history,
            &req.message,
            st.max_chars,
        );
        if !card.post_history_instructions.trim().is_empty() {
            msgs.push(zhuo_llm::Msg::system(
                card.post_history_instructions.clone(),
            ));
        }
        (card.id, active_chat.map(|c| c.id), msgs, hits)
    };

    // 2. 通道 + 第一帧：本次注入的记忆（前端展示"已注入 N 条相关记忆"）
    let (tx, rx) = unbounded_channel::<String>();
    let _ = tx.send(to_payload(&mem_hits).to_string());

    // 3. 单段 spawn：调 Ollama 流式，token 转发；成功后消息与 token 用量入库
    let connection = connection_settings(&st);
    let base = connection.base_url;
    let key = connection.api_key;
    let model = connection.model;
    let st2 = st.clone();
    let input = if req.display_message.trim().is_empty() {
        req.message.clone()
    } else {
        req.display_message.clone()
    };
    let regex_scripts: Vec<RegexScript> = st
        .memory
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_setting("regex_scripts")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();

    tokio::spawn(async move {
        let cfg = StreamConfig {
            base_url: base.as_str(),
            api_key: key.as_str(),
            model: model.as_str(),
            include_usage: st2.include_usage,
            dump_path: None,
            response_format: None,
            temperature: Some(connection.temperature),
            max_tokens: Some(connection.max_tokens),
            top_p: Some(connection.top_p),
            frequency_penalty: Some(connection.frequency_penalty),
            presence_penalty: Some(connection.presence_penalty),
            repetition_penalty: Some(connection.repetition_penalty),
            // 主模型按 ZHUO_MAIN_NUM_GPU 分配 GPU 层数（默认 99）
            num_gpu: Some(st2.main_num_gpu),
            ollama_native: st2.ollama_native,
        };
        // token 转发；send 失败 = 客户端已断开（点「停止生成」或关页面）→ 返回 false
        // 中断上游 Ollama 请求，已生成的部分内容仍会入库
        let result = zhuo_llm::stream_chat_ex(&cfg, &msgs, |tok| {
            tx.send(json!({ "token": tok }).to_string()).is_ok()
        })
        .await;

        match result {
            Ok(StreamOutcome { content, usage, .. }) if !content.is_empty() => {
                let processed = apply_output_regex(&content, &regex_scripts);
                let mem = st2.memory.lock().unwrap_or_else(|e| e.into_inner());
                let add_user = match active_chat.as_deref() {
                    Some(id) => mem.add_chat_message(id, &card_key, "user", &input),
                    None => mem.add_message(&card_key, "user", &input),
                };
                if let Err(e) = add_user {
                    eprintln!("[memory] add user message failed: {e:#}");
                }
                let add_assistant = match active_chat.as_deref() {
                    Some(id) => mem.add_chat_message(id, &card_key, "assistant", &content),
                    None => mem.add_message(&card_key, "assistant", &content),
                };
                if let Err(e) = add_assistant {
                    eprintln!("[memory] add assistant message failed: {e:#}");
                }
                drop(mem);
                let _ =
                    tx.send(json!({"processed": processed, "raw_content": content}).to_string());
                if let Some(u) = usage {
                    let _ = tx.send(json!({ "usage": u }).to_string());
                }

                // 后台记忆提取：满 N 轮未处理即触发（per-card 防并发；失败不影响主流程）
                if st2.summary_enabled() && st2.try_begin_extract(&card_key) {
                    let st3 = st2.clone();
                    let cn = card_key.clone();
                    tokio::spawn(async move {
                        let r = crate::summarize::run_extraction(&st3, &cn).await;
                        st3.end_extract(&cn);
                        match r {
                            Ok(Some(s)) => eprintln!("[summarize] {cn}: 自动提取 -> {s}"),
                            Ok(None) => {}
                            Err(e) => eprintln!("[summarize] {cn}: 提取失败(已跳过): {e:#}"),
                        }
                    });
                }
            }
            Ok(_) => {
                let _ = tx.send(json!({ "error": "空回复" }).to_string());
            }
            Err(e) => {
                let _ = tx.send(json!({ "error": format!("{e:#}") }).to_string());
            }
        }
        let _ = tx.send("[DONE]".into());
    });

    let stream = unfold_rx(rx);
    Sse::new(stream)
}

fn apply_output_regex(input: &str, scripts: &[RegexScript]) -> String {
    scripts
        .iter()
        .filter(|s| {
            !s.disabled && !s.prompt_only && (s.placement.is_empty() || s.placement.contains(&2))
        })
        .fold(input.to_string(), |text, script| {
            let raw = script.find_regex.trim();
            let (pattern, flags) = if raw.starts_with('/') && raw[1..].rfind('/').is_some() {
                let end = raw.rfind('/').unwrap();
                (&raw[1..end], &raw[end + 1..])
            } else {
                (raw, "")
            };
            let mut builder = regex::RegexBuilder::new(pattern);
            builder
                .case_insensitive(flags.contains('i'))
                .multi_line(flags.contains('m'))
                .dot_matches_new_line(flags.contains('s'));
            let Ok(re) = builder.build() else { return text };
            let replacement = script
                .replace_string
                .replace("{{match}}", "$0")
                .replace("$&", "$0");
            if flags.contains('g') {
                re.replace_all(&text, replacement.as_str()).to_string()
            } else {
                re.replace(&text, replacement.as_str()).to_string()
            }
        })
}

/// GET /api/history?card= —— 从 SQLite 读对话历史（card 传 id 或名称均可）
pub async fn history(
    State(st): State<ServerState>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    let result = if let Some(chat) = q.get("chat") {
        mem.chat_history(chat, 200)
    } else {
        let key = st.card_key(&q.get("card").cloned().unwrap_or_default());
        mem.history(&key, 200)
    };
    match result {
        Ok(msgs) => Json(json!(msgs
            .iter()
            .map(|m| json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>())),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

pub async fn list_chats(
    State(st): State<ServerState>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let card = q.get("card").map(|s| st.card_key(s));
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.list_chats(card.as_deref()) {
        Ok(v) => Json(json!(v)),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

pub async fn create_chat(
    State(st): State<ServerState>,
    Json(req): Json<CreateChatRequest>,
) -> Json<serde_json::Value> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let card = match st.find_card(&req.card) {
        Ok(v) => v,
        Err(e) => return Json(json!({"error": format!("{e:#}")})),
    };
    let id = format!(
        "chat-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let title = if req.title.trim().is_empty() {
        "新对话"
    } else {
        req.title.trim()
    };
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.create_chat(&id, &card.id, title) {
        Ok(chat) => {
            if !card.first_mes.trim().is_empty() {
                let _ = mem.add_chat_message(&id, &card.id, "assistant", &card.first_mes);
            }
            Json(json!(chat))
        }
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

pub async fn rename_chat(
    State(st): State<ServerState>,
    Path(id): Path<String>,
    Json(req): Json<RenameChatRequest>,
) -> Json<serde_json::Value> {
    if req.title.trim().is_empty() {
        return Json(json!({"error":"标题不能为空"}));
    }
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.rename_chat(&id, req.title.trim()) {
        Ok(n) => Json(json!({"ok": n > 0})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

pub async fn delete_chat(
    State(st): State<ServerState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.delete_chat(&id) {
        Ok(n) => Json(json!({"ok": n > 0})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

pub async fn get_connection(State(st): State<ServerState>) -> Json<serde_json::Value> {
    let mut cfg = connection_settings(&st);
    // Ollama 本地端点的 "ollama" 也是已配置的 key，不能再被当成「未配置」占位符。
    let configured = !cfg.api_key.trim().is_empty();
    cfg.api_key = if configured {
        "••••••••".into()
    } else {
        String::new()
    };
    Json(json!({
        "connection": cfg,
        "api_key_configured": configured,
        "api_key_source": api_key_source(&st),
    }))
}

pub async fn set_connection(
    State(st): State<ServerState>,
    Json(mut cfg): Json<ConnectionSettings>,
) -> Json<serde_json::Value> {
    if cfg.base_url.trim().is_empty() || cfg.model.trim().is_empty() {
        return Json(json!({"error":"Base URL 和模型不能为空"}));
    }
    // 前端回填的是掩码或空串 → 保留实际生效的 key（.env 优先），不要把它抹掉。
    if cfg.api_key.trim().is_empty() || cfg.api_key == "••••••••" {
        cfg.api_key = connection_settings(&st).api_key;
    }
    cfg.temperature = cfg.temperature.clamp(0.0, 2.0);
    cfg.max_tokens = cfg.max_tokens.clamp(1, 32768);
    cfg.top_p = cfg.top_p.clamp(0.0, 1.0);
    cfg.frequency_penalty = cfg.frequency_penalty.clamp(-2.0, 2.0);
    cfg.presence_penalty = cfg.presence_penalty.clamp(-2.0, 2.0);
    cfg.repetition_penalty = cfg.repetition_penalty.clamp(0.0, 2.0);
    let raw = serde_json::to_string(&cfg).unwrap_or_default();
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.set_setting("connection", &raw) {
        Ok(()) => Json(json!({"ok":true})),
        Err(e) => Json(json!({"error":format!("{e:#}")})),
    }
}

/// POST /api/reset —— 清空对话历史（可选连记忆一起清）
pub async fn reset(
    State(st): State<ServerState>,
    Json(req): Json<ResetRequest>,
) -> Json<serde_json::Value> {
    let key = st.card_key(&req.card);
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    let r = mem.clear_messages(&key);
    let m = if req.memories {
        mem.clear_memories(&key)
    } else {
        Ok(0)
    };
    match (r, m) {
        (Ok(a), Ok(b)) => Json(json!({"ok": true, "messages_cleared": a, "memories_cleared": b})),
        (Err(e), _) | (_, Err(e)) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// DELETE /api/history?card= —— 新建会话：只清当前对话记录，记忆与状态保留。
/// （POST /api/reset 是老入口，保留兼容；前端「新会话」按钮走这个。）
pub async fn clear_history(
    State(st): State<ServerState>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let key = st.card_key(&q.get("card").cloned().unwrap_or_default());
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.clear_messages(&key) {
        Ok(n) => Json(json!({"ok": true, "messages_cleared": n})),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// POST /api/memories —— 手工喂一条记忆卡片（种子/测试用）
pub async fn add_memory(
    State(st): State<ServerState>,
    Json(req): Json<AddMemoryRequest>,
) -> Json<serde_json::Value> {
    let key = st.card_key(&req.card);
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.add_memory(&key, &req.summary, &req.keywords, req.source_round) {
        Ok(id) => {
            Json(json!({"ok": true, "id": id, "total": mem.count_memories(&key).unwrap_or(0)}))
        }
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// GET /api/memories?card=&q=&limit= —— 调试视图。
/// 带 q：FTS5 关键词检索（bm25 排序）；不带 q：最近记忆列表（调试面板用）。
pub async fn search_memories(
    State(st): State<ServerState>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let key = st.card_key(&q.get("card").cloned().unwrap_or_default());
    let query = q.get("q").cloned().unwrap_or_default();
    let limit: usize = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(20);
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    if query.is_empty() {
        let list = mem.list_memories(&key, limit).unwrap_or_default();
        return Json(json!({
            "total": mem.count_memories(&key).unwrap_or(0),
            "unprocessed": mem.count_unprocessed(&key).unwrap_or(0),
            "memories": list
                .iter()
                .map(|h| json!({
                    "id": h.id,
                    "summary": h.summary,
                    "keywords": h.keywords,
                    "source_round": h.source_round,
                    "created_at": h.created_at,
                }))
                .collect::<Vec<_>>(),
        }));
    }
    match mem.search(&key, &query, limit) {
        Ok(hits) => Json(to_payload(&hits)),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// POST /api/summarize —— 手动触发一次后台提取（调试面板/测试用；成功返回总结文本）
pub async fn summarize(
    State(st): State<ServerState>,
    Json(req): Json<SummarizeRequest>,
) -> Json<serde_json::Value> {
    if !st.summary_enabled() {
        return Json(json!({"error": "ZHUO_SUMMARY_MODEL 未设置，自动提取关闭"}));
    }
    let key = st.card_key(&req.card);
    // 与后台自动提取共用 per-card 防并发：已在提取中直接告知，避免双写重复记忆
    if !st.try_begin_extract(&key) {
        return Json(json!({"ok": false, "skipped": "该卡正在提取中，请稍后再试"}));
    }
    let r = crate::summarize::run_extraction(&st, &key).await;
    st.end_extract(&key);
    match r {
        Ok(Some(s)) => Json(json!({"ok": true, "summary": s})),
        Ok(None) => Json(json!({"ok": false, "skipped": "未处理消息不足或模型输出不可解析"})),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// GET /api/state?card= —— 读角色状态（无记录时 state 为 null）
pub async fn get_state(
    State(st): State<ServerState>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let key = st.card_key(&q.get("card").cloned().unwrap_or_default());
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.get_state(&key) {
        Ok(Some(v)) => Json(json!({"card": key, "state": v})),
        Ok(None) => Json(json!({"card": key, "state": null})),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// PUT /api/state —— 手动更新角色状态（蓝图 Phase 4：状态自动提取暂不做，先手动验证注入）
pub async fn set_state(
    State(st): State<ServerState>,
    Json(req): Json<SetStateRequest>,
) -> Json<serde_json::Value> {
    if !req.state.is_object() {
        return Json(
            json!({"error": "state 必须是 JSON 对象，如 {\"背包\":[\"木剑\"],\"位置\":\"冬堡学院\"}"}),
        );
    }
    let key = st.card_key(&req.card);
    let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
    match mem.set_state(&key, &req.state) {
        Ok(()) => Json(json!({"ok": true, "card": key, "state": req.state})),
        Err(e) => Json(json!({"error": format!("{e:#}")})),
    }
}

/// GET / —— 健康检查
pub async fn index() -> Json<serde_json::Value> {
    Json(json!({"status": "zhuo-server ok"}))
}

fn sse_error(msg: String) -> Sse<SseStream> {
    let (tx, rx) = unbounded_channel::<String>();
    let _ = tx.send(json!({ "error": msg }).to_string());
    let _ = tx.send("[DONE]".into());
    Sse::new(unfold_rx(rx))
}

/// unbounded_channel → SSE 流：同步 send 在 async runtime 内安全（非 blocking_send）。
fn unfold_rx(rx: UnboundedReceiver<String>) -> SseStream {
    Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|payload| (Ok(Event::default().data(payload)), rx))
    }))
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn st_output_regex_honors_flags_placement_and_capture_groups() {
        let scripts = vec![RegexScript {
            script_name: "clean".into(),
            find_regex: "/foo (bar)/gi".into(),
            replace_string: "[$1]".into(),
            placement: vec![2],
            disabled: false,
            prompt_only: false,
        }];
        assert_eq!(
            apply_output_regex("Foo bar, foo BAR", &scripts),
            "[bar], [BAR]"
        );
        let ignored = RegexScript {
            placement: vec![1],
            ..scripts[0].clone()
        };
        assert_eq!(apply_output_regex("foo bar", &[ignored]), "foo bar");
    }
}
