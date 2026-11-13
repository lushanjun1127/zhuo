//! 服务端状态与配置：全部来自环境变量，默认值对齐本地 Ollama。

use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zhuo_card::Card;
use zhuo_memory::MemoryStore;

#[derive(Clone)]
pub struct ServerState {
    pub base_url: Arc<String>,
    pub api_key: Arc<String>,
    pub model: Arc<String>,
    pub cards_dir: Arc<PathBuf>,
    pub world: Arc<String>,
    pub max_chars: usize,
    pub include_usage: bool,
    /// SQLite 记忆库（Connection 非 Sync，用 Mutex 短临界区保护）
    pub memory: Arc<Mutex<MemoryStore>>,
    /// 后台提取用的 1B 小模型（ZHUO_SUMMARY_MODEL；为空 = 自动提取关闭）
    pub summary_model: Arc<String>,
    /// 触发提取的未处理消息条数阈值（ZHUO_SUMMARY_ROUNDS，默认 8）
    pub summary_rounds: usize,
    /// per-card 提取防并发：正在提取的卡集合
    extracting: Arc<Mutex<HashSet<String>>>,
    /// 主模型 GPU 层数（ZHUO_MAIN_NUM_GPU，默认 99 = 尽量全上 GPU）
    pub main_num_gpu: i64,
    /// 提取模型 GPU 层数（ZHUO_SUMMARY_NUM_GPU，默认 0 = 强制纯 CPU）
    pub summary_num_gpu: i64,
    /// 走 Ollama 原生 /api/chat（默认 true），否则走 /v1/chat/completions
    pub ollama_native: bool,
    /// .env 是否显式配置了 ZHUO_API_KEY（决定连接设置的优先级与来源标注）
    pub env_api_key: bool,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

/// 解析 GPU 层数；未设置 / 非法值一律回落默认值。
fn parse_num_gpu(raw: Option<String>, default: i64) -> i64 {
    raw.and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

fn env_flag(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => default,
    }
}

impl ServerState {
    pub fn from_env() -> Result<Self> {
        let cards_dir = PathBuf::from(env_or("ZHUO_CARDS_DIR", "examples"));
        let world_path = PathBuf::from(env_or("ZHUO_WORLD", "examples/world.json"));
        let world = std::fs::read_to_string(&world_path)
            .map(|raw| {
                serde_json::from_str::<serde_json::Value>(&raw)
                    .map(|v| crate::prompt::world_text(&v))
                    .unwrap_or_else(|_| raw)
            })
            .unwrap_or_default();

        let db_path = env_or("ZHUO_DB", zhuo_memory::DEFAULT_DB);
        let memory = MemoryStore::open(&db_path)?;
        memory.init_schema()?;

        Ok(Self {
            base_url: Arc::new(env_or("ZHUO_API_BASE", zhuo_llm::DEFAULT_BASE)),
            api_key: Arc::new(env_or("ZHUO_API_KEY", zhuo_llm::DEFAULT_API_KEY)),
            model: Arc::new(env_or("ZHUO_MODEL", "qwen2.5-coder:3b")),
            cards_dir: Arc::new(cards_dir),
            world: Arc::new(world),
            max_chars: env_or("ZHUO_MAX_CHARS", "12000").parse().unwrap_or(12_000),
            include_usage: std::env::var("ZHUO_NO_USAGE").is_err(),
            memory: Arc::new(Mutex::new(memory)),
            summary_model: Arc::new(std::env::var("ZHUO_SUMMARY_MODEL").unwrap_or_default()),
            summary_rounds: std::env::var("ZHUO_SUMMARY_ROUNDS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8),
            extracting: Arc::new(Mutex::new(HashSet::new())),
            main_num_gpu: parse_num_gpu(std::env::var("ZHUO_MAIN_NUM_GPU").ok(), 99),
            summary_num_gpu: parse_num_gpu(std::env::var("ZHUO_SUMMARY_NUM_GPU").ok(), 0),
            ollama_native: env_flag("ZHUO_OLLAMA_NATIVE", true),
            env_api_key: std::env::var("ZHUO_API_KEY")
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false),
        })
    }

    /// 自动提取是否开启（ZHUO_SUMMARY_MODEL 已设置）
    pub fn summary_enabled(&self) -> bool {
        !self.summary_model.is_empty()
    }

    /// 尝试占用某卡的提取通道；已在提取中返回 false（防并发重复触发）
    pub fn try_begin_extract(&self, card: &str) -> bool {
        self.extracting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(card.to_string())
    }

    pub fn end_extract(&self, card: &str) {
        self.extracting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(card);
    }

    /// 按 id → 显示名 → 文件名解析卡（同名卡靠 id 消歧）。
    fn resolve_card(&self, s: &str) -> Option<Card> {
        let rd = std::fs::read_dir(&*self.cards_dir).ok()?;
        for e in rd.flatten() {
            let p = e.path();
            if !matches!(
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref(),
                Some("json" | "png")
            ) {
                continue;
            }
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem == "world" {
                continue; // world.json 不是卡
            }
            if let Ok(c) = zhuo_card::load_card(&p) {
                if c.id == s || c.name == s || stem == s {
                    return Some(c);
                }
            }
        }
        None
    }

    /// 找卡（不存在则报错）。chat 主路径用。
    pub fn find_card(&self, s: &str) -> Result<Card> {
        self.resolve_card(s)
            .ok_or_else(|| anyhow::anyhow!("card not found: {s}"))
    }

    /// 解析存储键（卡 id）；找不到卡时原样返回字符串（兼容旧数据键）。
    pub fn card_key(&self, s: &str) -> String {
        self.resolve_card(s)
            .map(|c| c.id)
            .unwrap_or_else(|| s.to_string())
    }

    pub fn list_cards(&self) -> Vec<Card> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&*self.cards_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !matches!(
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref(),
                    Some("json" | "png")
                ) {
                    continue;
                }
                if p.file_stem().and_then(|s| s.to_str()) == Some("world") {
                    continue;
                }
                if let Ok(c) = zhuo_card::load_card(&p) {
                    out.push(c);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn card_source_path(&self, id: &str) -> Option<PathBuf> {
        std::fs::read_dir(&*self.cards_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                matches!(
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref(),
                    Some("json" | "png")
                ) && zhuo_card::load_card(p).map(|c| c.id == id).unwrap_or(false)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_split_defaults_and_overrides() {
        assert_eq!(parse_num_gpu(None, 99), 99, "主模型默认全量上 GPU");
        assert_eq!(parse_num_gpu(None, 0), 0, "提取模型默认强制 CPU");
        assert_eq!(parse_num_gpu(Some(" 0 ".into()), 99), 0);
        assert_eq!(parse_num_gpu(Some("24".into()), 99), 24);
        assert_eq!(parse_num_gpu(Some("bad".into()), 0), 0, "非法值回落默认");
    }

    #[test]
    fn native_channel_defaults_on() {
        assert!(env_flag("ZHUO_NON_EXISTENT_FLAG_XYZ", true));
        assert!(!env_flag("ZHUO_NON_EXISTENT_FLAG_XYZ", false));
    }
}
