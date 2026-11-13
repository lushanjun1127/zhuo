//! zhuo-server：axum 后端，对接 React 前端与本地 Ollama。
//!
//! 环境变量（全部可选）：
//!   ZHUO_API_BASE   推理端点（默认 zhuo_llm::DEFAULT_BASE = http://localhost:11434/v1）
//!   ZHUO_API_KEY    API key（默认 "ollama"，Ollama 不校验）
//!   ZHUO_MODEL      模型名
//!   ZHUO_CARDS_DIR  角色卡目录（默认 examples/）
//!   ZHUO_WORLD      世界设定 JSON（默认 examples/world.json）
//!   ZHUO_DB         SQLite 记忆库路径（默认 zhuo.db，蓝图低配轨）
//!   ZHUO_MAX_CHARS  上下文字符上限（默认 12000）
//!   ZHUO_PORT       监听端口（默认 1127）
//!   ZHUO_NO_USAGE=1 不请求 stream_options.include_usage

mod prompt;
mod routes;
mod state;
mod summarize;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{middleware, Router};

/// 纯 Web 形态下唯一允许的来源：Vite 前端（npm run start 打开的地址）。
/// 必须用 http://localhost:3000 访问；127.0.0.1:3000 属于不同 Origin，会被拒绝。
const ALLOWED_ORIGIN: &str = "http://localhost:3000";

/// 手写 CORS（不引 tower-http，尊重「能自己写 50 行就不引依赖」红线）。
/// - 带 Origin 且不是白名单：直接 403，连预检也不放行（拒绝其他任何源）；
/// - 带白名单 Origin：只回该具体源 + Vary: Origin，不再用通配符；
/// - 不带 Origin：同源 GET / 健康检查 / 本地脚本，按同源请求放行且不追加 CORS 头。
async fn cors(req: Request<Body>, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    if let Some(o) = origin.as_deref() {
        if o != ALLOWED_ORIGIN {
            return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
        }
    }

    let mut res = if req.method() == Method::OPTIONS {
        StatusCode::OK.into_response()
    } else {
        next.run(req).await
    };
    let h = res.headers_mut();
    if origin.is_some() {
        h.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static(ALLOWED_ORIGIN),
        );
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,POST,PUT,PATCH,DELETE,OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type"),
    );
    res
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let st = state::ServerState::from_env()?;
    let port: u16 = std::env::var("ZHUO_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1127);

    println!("酌 / zhuo-server");
    println!("  推理端点   : {}", st.base_url);
    println!("  模型       : {}（可用 ZHUO_MODEL 覆盖）", st.model);
    println!("  角色卡目录 : {:?}", st.cards_dir);
    println!("  记忆库     : SQLite FTS5 + jieba（ZHUO_DB，默认 zhuo.db）");
    if st.summary_enabled() {
        println!(
            "  提取模型   : {}（每 {} 条未处理消息自动总结一轮）",
            st.summary_model, st.summary_rounds
        );
    } else {
        println!("  提取模型   : 未启用（设置 ZHUO_SUMMARY_MODEL 开启后台自动记忆提取）");
    }
    if std::env::var("ZHUO_MODEL").is_err() {
        println!("  [提示] 未设置 ZHUO_MODEL，默认 qwen2.5-coder:3b；本机缓存的完整名可用 ollama list 查看");
    }
    println!(
        "  GPU 分流     : 主模型 num_gpu={} / 提取模型 num_gpu={}（0 = 强制 CPU）",
        st.main_num_gpu, st.summary_num_gpu
    );
    println!(
        "  推理通道     : {}",
        if st.ollama_native {
            "Ollama 原生 /api/chat（options.num_gpu 生效）"
        } else {
            "OpenAI 兼容 /v1/chat/completions"
        }
    );

    let app = Router::new()
        .route("/", get(routes::index))
        .route("/api/cards", get(routes::cards))
        .route("/api/cards/import", post(routes::import_card))
        .route("/api/cards/:id/export", get(routes::export_card))
        .route("/api/compatibility", get(routes::compatibility))
        .route(
            "/api/compatibility/worldbook",
            post(routes::import_worldbook),
        )
        .route("/api/compatibility/preset", post(routes::import_preset))
        .route("/api/compatibility/regex", post(routes::import_regex))
        .route("/api/chat", post(routes::chat))
        .route(
            "/api/chats",
            get(routes::list_chats).post(routes::create_chat),
        )
        .route(
            "/api/chats/:id",
            axum::routing::patch(routes::rename_chat).delete(routes::delete_chat),
        )
        .route(
            "/api/connection",
            get(routes::get_connection).put(routes::set_connection),
        )
        .route(
            "/api/history",
            get(routes::history).delete(routes::clear_history),
        )
        .route("/api/reset", post(routes::reset))
        .route(
            "/api/memories",
            post(routes::add_memory).get(routes::search_memories),
        )
        .route("/api/summarize", post(routes::summarize))
        .route("/api/state", get(routes::get_state).put(routes::set_state))
        .layer(middleware::from_fn(cors))
        .with_state(st);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("  监听       : http://127.0.0.1:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
