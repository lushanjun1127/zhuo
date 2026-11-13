//! 低配轨记忆系统：SQLite + FTS5 + jieba-rs，零显存占用。
//!
//! 设计（对应《酌》架构蓝图 3.1）：
//! - **Message 表**：对话历史持久化（替代旧 JSON 落盘，单一事实来源）。
//! - **Memory 表 + FTS5 虚拟表**：压缩后的结构化记忆卡片。写入前用 jieba 分词，
//!   检索时同样分词后拼 FTS5 MATCH 语法，`bm25()` 排序取 Top-K。
//! - 检索结果作为**独立文本块**插入 System Prompt 与聊天历史之间，生成后即丢弃，
//!   不污染永久聊天记录。
//!
//! 全部在进程内 SQLite 完成，纯 CPU，毫秒级检索数万条记忆。

use anyhow::{Context, Result};
use jieba_rs::Jieba;
use rusqlite::Connection;
use std::path::Path;
use std::sync::OnceLock;
use zhuo_llm::Msg;

/// 默认数据库路径：蓝图约定的 zhuo.db（可用 ZHUO_DB 覆盖）
pub const DEFAULT_DB: &str = "zhuo.db";

/// 建表 SQL：Message 表 + Memory 表 + FTS5 虚拟表。
/// messages.processed：后台记忆提取的游标（0=未处理，1=已被总结并写入记忆库）。
/// memories_fts 是独立内容表（非 external content），body 为 jieba 分词后的空格分隔文本，
/// `mem_id`/`card_name` 为 UNINDEXED 元数据列，仅供过滤与回表关联。
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    card_name  TEXT    NOT NULL,
    role       TEXT    NOT NULL CHECK (role IN ('user','assistant','system')),
    content    TEXT    NOT NULL,
    processed  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_messages_card ON messages(card_name, id);

CREATE TABLE IF NOT EXISTS memories (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    card_name    TEXT    NOT NULL,
    summary      TEXT    NOT NULL,
    keywords     TEXT    NOT NULL DEFAULT '',
    source_round INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_memories_card ON memories(card_name, id);

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    mem_id    UNINDEXED,
    card_name UNINDEXED,
    body,
    tokenize = 'unicode61'
);

CREATE TABLE IF NOT EXISTS character_state (
    card_name  TEXT PRIMARY KEY,
    state_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS chats (
    id          TEXT PRIMARY KEY,
    card_name   TEXT NOT NULL,
    title       TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_chats_card ON chats(card_name, updated_at DESC);

CREATE TABLE IF NOT EXISTS app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
"#;

/// 旧库迁移：给已存在的 messages 表补 processed 列（幂等）。
pub const MIGRATIONS: &[&str] = &[
    "ALTER TABLE messages ADD COLUMN processed INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE messages ADD COLUMN chat_id TEXT NOT NULL DEFAULT ''",
];

fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

/// 中文/无意义虚词停用表：分词后过滤，避免 FTS5 查询被噪音 token 稀释。
const STOPWORDS: &[&str] = &[
    "的",
    "了",
    "吗",
    "呢",
    "吧",
    "啊",
    "呀",
    "哦",
    "嗯",
    "是",
    "在",
    "和",
    "与",
    "就",
    "都",
    "也",
    "很",
    "要",
    "有",
    "不",
    "这",
    "那",
    "我",
    "你",
    "他",
    "她",
    "它",
    "什么",
    "怎么",
    "怎么样",
    "如何",
    "请问",
    "一下",
    "些",
    "个",
];

fn is_noise(token: &str) -> bool {
    // 纯标点/符号/空白
    if token.chars().all(|c| {
        c.is_whitespace()
            || c.is_ascii_punctuation()
            || matches!(c as u32,
            0x3000..=0x303F | 0xFF00..=0xFFEF | 0x2018..=0x201D | 0x2026 | 0x2014)
    }) {
        return true;
    }
    STOPWORDS.contains(&token)
}

/// jieba 搜索级分词 + 去噪，输出空格分隔的 token 流（FTS5 unicode61 以空格切词）。
pub fn seg(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for t in jieba().cut_for_search(text, true) {
        let t = t.trim();
        if !t.is_empty() && !is_noise(t) && !out.contains(&t) {
            out.push(t);
        }
    }
    out.join(" ")
}

/// 把分好的 token 流拼成 FTS5 MATCH 查询语法（OR 语义，bm25 排序自动权衡相关度）。
/// token 用双引号包裹（FTS5 字符串语法），内部引号按规则转义。
pub fn fts5_query(text: &str) -> String {
    let tokens: Vec<String> = seg(text)
        .split(' ')
        .filter(|s| !s.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    tokens.join(" OR ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHit {
    pub id: i64,
    pub summary: String,
    pub keywords: String,
    pub source_round: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ChatRecord {
    pub id: String,
    pub card: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    /// 打开（或创建）数据库并建表。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create db dir: {}", parent.display()))?;
            }
        }
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite: {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .context("configure sqlite concurrency")?;
        let me = Self { conn };
        me.init_schema()?;
        Ok(me)
    }

    /// 内存库（测试/临时会话用）
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("open in-memory sqlite")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .context("configure in-memory sqlite")?;
        let me = Self { conn };
        me.init_schema()?;
        Ok(me)
    }

    pub fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(SCHEMA_SQL)
            .context("init zhuo schema (需 FTS5 支持)")?;
        // 旧库迁移：约定格式 "ALTER TABLE <table> ADD COLUMN <col> <rest>"，幂等
        for m in MIGRATIONS {
            let toks: Vec<&str> = m.split_whitespace().collect();
            if toks.len() < 6 || !toks[0].eq_ignore_ascii_case("ALTER") {
                continue;
            }
            let table = toks[2];
            let col = toks[5];
            let have: i64 = self.conn.query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                [col],
                |r| r.get(0),
            )?;
            if have == 0 {
                self.conn
                    .execute(m, [])
                    .with_context(|| format!("migration: {m}"))?;
            }
        }
        // Legacy databases stored one implicit conversation per character. Promote those rows
        // to an explicit chat exactly once; new rows already carry a non-empty chat_id.
        self.conn.execute_batch(
            "INSERT OR IGNORE INTO chats (id, card_name, title, created_at, updated_at)
             SELECT 'legacy-' || card_name, card_name, '迁移的对话', MIN(created_at), MAX(created_at)
             FROM messages WHERE chat_id = '' GROUP BY card_name;
             UPDATE messages SET chat_id = 'legacy-' || card_name WHERE chat_id = '';",
        ).context("migrate legacy conversations")?;
        Ok(())
    }

    // ---------- Message 表：对话历史 ----------

    pub fn add_message(&self, card_name: &str, role: &str, content: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO messages (card_name, role, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![card_name, role, content],
            )
            .context("insert message")?;
        Ok(())
    }

    /// 某角色的最近 `limit` 条消息（按时间正序返回，可直接进 prompt）
    pub fn history(&self, card_name: &str, limit: usize) -> Result<Vec<Msg>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM (
                 SELECT role, content, id FROM messages
                 WHERE card_name = ?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![card_name, limit as i64], |r| {
            Ok(Msg::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn clear_messages(&self, card_name: &str) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM messages WHERE card_name = ?1", [card_name])
            .context("clear messages")?)
    }

    // ---------- Chat sessions ----------

    pub fn create_chat(&self, id: &str, card: &str, title: &str) -> Result<ChatRecord> {
        self.conn
            .execute(
                "INSERT INTO chats (id, card_name, title) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, card, title],
            )
            .context("create chat")?;
        self.get_chat(id)?.context("created chat missing")
    }

    pub fn get_chat(&self, id: &str) -> Result<Option<ChatRecord>> {
        self.conn
            .query_row(
                "SELECT id, card_name, title, created_at, updated_at FROM chats WHERE id = ?1",
                [id],
                |r| {
                    Ok(ChatRecord {
                        id: r.get(0)?,
                        card: r.get(1)?,
                        title: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("get chat")
    }

    pub fn list_chats(&self, card: Option<&str>) -> Result<Vec<ChatRecord>> {
        let sql = if card.is_some() {
            "SELECT id, card_name, title, created_at, updated_at FROM chats WHERE card_name = ?1 ORDER BY updated_at DESC"
        } else {
            "SELECT id, card_name, title, created_at, updated_at FROM chats ORDER BY updated_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let make = |r: &rusqlite::Row<'_>| {
            Ok(ChatRecord {
                id: r.get(0)?,
                card: r.get(1)?,
                title: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        };
        let rows = if let Some(card) = card {
            stmt.query_map([card], make)?
        } else {
            stmt.query_map([], make)?
        };
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn rename_chat(&self, id: &str, title: &str) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE chats SET title = ?2, updated_at = datetime('now','localtime') WHERE id = ?1",
            rusqlite::params![id, title],
        )?)
    }

    pub fn delete_chat(&self, id: &str) -> Result<usize> {
        self.conn
            .execute("DELETE FROM messages WHERE chat_id = ?1", [id])?;
        Ok(self.conn.execute("DELETE FROM chats WHERE id = ?1", [id])?)
    }

    pub fn add_chat_message(
        &self,
        chat_id: &str,
        card: &str,
        role: &str,
        content: &str,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO messages (card_name, chat_id, role, content) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![card, chat_id, role, content],
            )
            .context("insert chat message")?;
        self.conn.execute(
            "UPDATE chats SET updated_at = datetime('now','localtime') WHERE id = ?1",
            [chat_id],
        )?;
        Ok(())
    }

    pub fn chat_history(&self, chat_id: &str, limit: usize) -> Result<Vec<Msg>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content FROM (SELECT role, content, id FROM messages WHERE chat_id = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![chat_id, limit as i64], |r| {
            Ok(Msg::new(r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn clear_chat_messages(&self, chat_id: &str) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM messages WHERE chat_id = ?1", [chat_id])?)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("get setting")
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = datetime('now','localtime')",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    // ---------- 未处理游标：后台记忆提取用 ----------

    /// 尚未被总结的消息条数
    pub fn count_unprocessed(&self, card_name: &str) -> Result<usize> {
        Ok(self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE card_name = ?1 AND processed = 0",
                [card_name],
                |r| r.get::<_, i64>(0),
            )
            .context("count unprocessed")? as usize)
    }

    /// 取最近 `limit` 条未处理消息（id 正序返回），供总结器拼 transcript。
    pub fn unprocessed(&self, card_name: &str, limit: usize) -> Result<Vec<(i64, Msg)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content FROM (
                 SELECT id, role, content FROM messages
                 WHERE card_name = ?1 AND processed = 0
                 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![card_name, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Msg::new(r.get::<_, String>(1)?, r.get::<_, String>(2)?),
            ))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 把 `upto_id`（含）之前的未处理消息标记为已总结
    pub fn mark_processed(&self, card_name: &str, upto_id: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute(
                "UPDATE messages SET processed = 1
                 WHERE card_name = ?1 AND processed = 0 AND id <= ?2",
                rusqlite::params![card_name, upto_id],
            )
            .context("mark processed")?)
    }

    /// 最近写入的记忆列表（调试面板用，id 倒序）
    pub fn list_memories(&self, card_name: &str, limit: usize) -> Result<Vec<MemoryHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, summary, keywords, source_round, created_at
             FROM memories WHERE card_name = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![card_name, limit as i64], |r| {
            Ok(MemoryHit {
                id: r.get(0)?,
                summary: r.get(1)?,
                keywords: r.get(2)?,
                source_round: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    // ---------- 角色状态机（蓝图 5.2）：手动更新，System 区注入 ----------

    /// 读角色状态（card 键可以是卡 id）；无记录返回 Ok(None)
    pub fn get_state(&self, card_key: &str) -> Result<Option<serde_json::Value>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT state_json FROM character_state WHERE card_name = ?1",
                [card_key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("get character_state")?;
        match raw {
            Some(s) => Ok(Some(
                serde_json::from_str(&s).context("parse character_state json")?,
            )),
            None => Ok(None),
        }
    }

    /// 写角色状态（upsert）。state 必须是 JSON 对象，否则拒绝写入。
    pub fn set_state(&self, card_key: &str, state: &serde_json::Value) -> Result<()> {
        if !state.is_object() {
            anyhow::bail!("character_state must be a JSON object");
        }
        let raw = serde_json::to_string(state).context("serialize state")?;
        self.conn
            .execute(
                "INSERT INTO character_state (card_name, state_json, updated_at)
                 VALUES (?1, ?2, datetime('now','localtime'))
                 ON CONFLICT(card_name) DO UPDATE SET
                     state_json = excluded.state_json,
                     updated_at = excluded.updated_at",
                rusqlite::params![card_key, raw],
            )
            .context("upsert character_state")?;
        Ok(())
    }

    // ---------- Memory 表 + FTS5：记忆卡片 ----------

    /// 写入一条记忆卡片。summary/keywords 会经 jieba 分词后进 FTS5 索引。
    pub fn add_memory(
        &self,
        card_name: &str,
        summary: &str,
        keywords: &str,
        source_round: i64,
    ) -> Result<i64> {
        if summary.trim().is_empty() {
            anyhow::bail!("memory summary is empty");
        }
        self.conn
            .execute(
                "INSERT INTO memories (card_name, summary, keywords, source_round)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![card_name, summary.trim(), keywords.trim(), source_round],
            )
            .context("insert memory")?;
        let id = self.conn.last_insert_rowid();
        let body = seg(&format!("{} {}", summary, keywords));
        self.conn
            .execute(
                "INSERT INTO memories_fts (mem_id, card_name, body) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, card_name, body],
            )
            .context("index memory (fts5)")?;
        Ok(id)
    }

    /// 关键词检索：jieba 分词 → FTS5 MATCH（OR）→ bm25 排序 Top-K。
    pub fn search(&self, card_name: &str, query: &str, limit: usize) -> Result<Vec<MemoryHit>> {
        let q = fts5_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.summary, m.keywords, m.source_round, m.created_at
             FROM memories_fts f
             JOIN memories m ON m.id = f.mem_id
             WHERE f.card_name = ?2 AND memories_fts MATCH ?1
             ORDER BY bm25(memories_fts)
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![q, card_name, limit as i64], |r| {
            Ok(MemoryHit {
                id: r.get(0)?,
                summary: r.get(1)?,
                keywords: r.get(2)?,
                source_round: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn count_memories(&self, card_name: &str) -> Result<usize> {
        Ok(self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE card_name = ?1",
                [card_name],
                |r| r.get::<_, i64>(0),
            )
            .context("count memories")? as usize)
    }

    pub fn clear_memories(&self, card_name: &str) -> Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM memories WHERE card_name = ?1", [card_name])
            .context("clear memories")?;
        self.conn
            .execute("DELETE FROM memories_fts WHERE card_name = ?1", [card_name])
            .context("clear memories_fts")?;
        Ok(n)
    }

    /// 记忆注入块：检索 Top-5 并渲染为「独立文本块」，拼进 System 与历史之间；
    /// 生成完毕即丢弃，不写入聊天记录。无相关记忆时返回 None（prompt 保持干净）。
    pub fn memory_block(
        &self,
        card_name: &str,
        query: &str,
        max_chars: usize,
    ) -> Result<Option<String>> {
        let hits = self.search(card_name, query, 5)?;
        if hits.is_empty() {
            return Ok(None);
        }
        let mut block =
            String::from("[相关记忆]\n以下是从长期记忆中检索到的、与当前对话相关的事实：\n");
        for h in &hits {
            let line = format!("- {}\n", h.summary);
            if block.chars().count() + line.chars().count() > max_chars {
                break;
            }
            block.push_str(&line);
        }
        Ok(Some(block))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &MemoryStore) {
        store
            .add_memory(
                "苏菲",
                "苏菲最喜欢吃山楂味的糖葫芦，冬天常在冬堡集市买。",
                "糖葫芦,零食",
                3,
            )
            .unwrap();
        store
            .add_memory(
                "苏菲",
                "冬堡常年下雪，冬天屋檐会挂冰凌。",
                "冬堡,天气,雪",
                5,
            )
            .unwrap();
        store
            .add_memory("苏菲", "冬堡学院图书馆的顶层藏着禁书区。", "学院,图书馆", 7)
            .unwrap();
    }

    #[test]
    fn schema_creates_fts5_tables() {
        let s = MemoryStore::open_in_memory().unwrap();
        assert!(s.count_memories("苏菲").unwrap() == 0);
    }

    #[test]
    fn seg_splits_chinese_and_drops_noise() {
        let toks = seg("你喜欢什么呢？糖葫芦！");
        assert!(toks.contains("糖葫芦"));
        assert!(toks.contains("喜欢"));
        assert!(!toks.contains("的"));
        assert!(!toks.contains("？"));
    }

    #[test]
    fn fts5_query_or_joins_quoted_tokens() {
        let q = fts5_query("冬堡下雪");
        assert!(q.contains('"'));
        assert!(q.contains(" OR "));
    }

    #[test]
    fn search_hits_relevant_memory() {
        let s = MemoryStore::open_in_memory().unwrap();
        seed(&s);
        // 语义相关：糖葫芦
        let hits = s.search("苏菲", "苏菲喜欢吃什么零食", 5).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].summary.contains("糖葫芦"));
        // 语义相关：天气/雪
        let hits = s.search("苏菲", "冬堡的天气怎么样", 5).unwrap();
        assert!(hits.iter().any(|h| h.summary.contains("下雪")));
        // 完全无关：不应命中任何记忆（回落路径）
        let hits = s.search("苏菲", "股票行情涨跌", 5).unwrap();
        assert!(hits.is_empty(), "无关查询不应命中: {:?}", hits);
    }

    #[test]
    fn memory_block_injects_and_falls_back() {
        let s = MemoryStore::open_in_memory().unwrap();
        seed(&s);
        let block = s.memory_block("苏菲", "苏菲喜欢吃什么零食", 600).unwrap();
        assert!(block.is_some());
        let b = block.unwrap();
        assert!(b.starts_with("[相关记忆]"));
        assert!(b.contains("糖葫芦"));

        // 无关查询 → None，prompt 不被噪音污染
        assert!(s
            .memory_block("苏菲", "股票行情涨跌", 600)
            .unwrap()
            .is_none());
    }

    #[test]
    fn memory_block_respects_max_chars() {
        let s = MemoryStore::open_in_memory().unwrap();
        for i in 0..10 {
            s.add_memory(
                "苏菲",
                &format!("记忆条目{i:04}：这是一条比较长的记忆内容用来撑字符数。"),
                "",
                i,
            )
            .unwrap();
        }
        let b = s.memory_block("苏菲", "记忆条目", 80).unwrap().unwrap();
        assert!(b.chars().count() <= 80 + 40); // 块头 + 截断容差
    }

    #[test]
    fn messages_roundtrip_and_clear() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.add_message("苏菲", "user", "你好").unwrap();
        s.add_message("苏菲", "assistant", "要来串糖葫芦吗？")
            .unwrap();
        s.add_message("冬堡", "user", "另一张卡").unwrap();
        let h = s.history("苏菲", 50).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].role, "user");
        assert_eq!(h[1].content, "要来串糖葫芦吗？");
        // limit 取最近 N 条
        let h = s.history("苏菲", 1).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].content, "要来串糖葫芦吗？");
        assert_eq!(s.clear_messages("苏菲").unwrap(), 2);
        assert!(s.history("苏菲", 50).unwrap().is_empty());
    }

    #[test]
    fn clear_memories_also_clears_fts() {
        let s = MemoryStore::open_in_memory().unwrap();
        seed(&s);
        assert_eq!(s.count_memories("苏菲").unwrap(), 3);
        s.clear_memories("苏菲").unwrap();
        assert_eq!(s.count_memories("苏菲").unwrap(), 0);
        assert!(s.search("苏菲", "糖葫芦", 5).unwrap().is_empty());
    }

    #[test]
    fn open_file_db_persists_across_connections() {
        let mut p = std::env::temp_dir();
        p.push(format!("zhuo_mem_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        {
            let s = MemoryStore::open(&p).unwrap();
            s.add_memory("苏菲", "苏菲的猫叫雪球，只吃蜂蜜。", "雪球,猫", 2)
                .unwrap();
        }
        let s2 = MemoryStore::open(&p).unwrap();
        let hits = s2.search("苏菲", "苏菲的猫吃什么", 5).unwrap();
        assert!(hits.iter().any(|h| h.summary.contains("雪球")));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn file_database_uses_wal_and_busy_timeout() {
        let mut p = std::env::temp_dir();
        p.push(format!("zhuo_wal_test_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);

        let store = MemoryStore::open(&p).unwrap();
        let journal_mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let busy_timeout: i64 = store
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(busy_timeout, 5000);
        drop(store);
        let _ = std::fs::remove_file(&p);
        let mut wal = p.as_os_str().to_os_string();
        wal.push("-wal");
        let _ = std::fs::remove_file(std::path::PathBuf::from(wal));
        let mut shm = p.as_os_str().to_os_string();
        shm.push("-shm");
        let _ = std::fs::remove_file(std::path::PathBuf::from(shm));
    }

    #[test]
    fn unprocessed_cursor_counts_and_marks() {
        let s = MemoryStore::open_in_memory().unwrap();
        s.add_message("苏菲", "user", "一").unwrap();
        s.add_message("苏菲", "assistant", "二").unwrap();
        s.add_message("冬堡", "user", "别的卡").unwrap();
        assert_eq!(s.count_unprocessed("苏菲").unwrap(), 2);

        let rows = s.unprocessed("苏菲", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.content, "一"); // id 正序

        // limit 取最近 N 条，但 id 仍正序
        let rows = s.unprocessed("苏菲", 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1.content, "二");

        // 标记到某 id 为止：只影响该卡的该 id 之前的未处理消息
        let max_id = rows[0].0;
        assert_eq!(s.mark_processed("苏菲", max_id).unwrap(), 2);
        assert_eq!(s.count_unprocessed("苏菲").unwrap(), 0);
        assert_eq!(s.count_unprocessed("冬堡").unwrap(), 1); // 别的卡不受影响
                                                             // 幂等
        assert_eq!(s.mark_processed("苏菲", max_id).unwrap(), 0);
    }

    #[test]
    fn list_memories_returns_recent_desc() {
        let s = MemoryStore::open_in_memory().unwrap();
        for i in 0..3 {
            s.add_memory("苏菲", &format!("记忆{i}"), "", i).unwrap();
        }
        let list = s.list_memories("苏菲", 2).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].summary, "记忆2"); // 最新在前
        assert_eq!(list[1].summary, "记忆1");
        assert!(!list[0].created_at.is_empty());
    }

    #[test]
    fn migration_adds_processed_to_legacy_db() {
        // 手工建一个"旧版"库（无 processed 列），再 open 应自动补列
        let mut p = std::env::temp_dir();
        p.push(format!("zhuo_mem_legacy_{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        {
            let conn = rusqlite::Connection::open(&p).unwrap();
            conn.execute_batch(
                "CREATE TABLE messages (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     card_name TEXT NOT NULL,
                     role TEXT NOT NULL,
                     content TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (datetime('now'))
                 );
                 INSERT INTO messages (card_name, role, content) VALUES ('苏菲','user','旧消息');",
            )
            .unwrap();
        }
        let s = MemoryStore::open(&p).unwrap();
        // 旧消息能读出来且未处理
        assert_eq!(s.count_unprocessed("苏菲").unwrap(), 1);
        let rows = s.unprocessed("苏菲", 10).unwrap();
        assert_eq!(rows[0].1.content, "旧消息");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn character_state_roundtrip_upsert_and_reject() {
        let s = MemoryStore::open_in_memory().unwrap();
        assert!(s.get_state("card_a").unwrap().is_none());
        s.set_state(
            "card_a",
            &serde_json::json!({"背包": ["木剑", "药水"], "位置": "冬堡学院", "好感度": 5}),
        )
        .unwrap();
        let st = s.get_state("card_a").unwrap().unwrap();
        assert_eq!(st["位置"], "冬堡学院");
        assert_eq!(st["背包"][0], "木剑");
        // upsert 覆盖
        s.set_state("card_a", &serde_json::json!({"位置": "集市"}))
            .unwrap();
        let st = s.get_state("card_a").unwrap().unwrap();
        assert_eq!(st["位置"], "集市");
        assert!(st.get("背包").is_none(), "整对象替换，不是合并");
        // 不同卡隔离
        assert!(s.get_state("card_b").unwrap().is_none());
        // 非对象拒绝
        assert!(s.set_state("card_a", &serde_json::json!([1, 2])).is_err());
    }

    #[test]
    fn chats_and_settings_roundtrip() {
        let s = MemoryStore::open_in_memory().unwrap();
        let chat = s.create_chat("chat-1", "card-a", "第一幕").unwrap();
        assert_eq!(chat.title, "第一幕");
        s.add_chat_message("chat-1", "card-a", "user", "你好")
            .unwrap();
        s.add_chat_message("chat-1", "card-a", "assistant", "欢迎")
            .unwrap();
        assert_eq!(s.chat_history("chat-1", 20).unwrap().len(), 2);
        s.rename_chat("chat-1", "重逢").unwrap();
        assert_eq!(s.list_chats(Some("card-a")).unwrap()[0].title, "重逢");
        s.set_setting("connection", "{\"model\":\"test\"}").unwrap();
        assert_eq!(
            s.get_setting("connection").unwrap().unwrap(),
            "{\"model\":\"test\"}"
        );
        assert_eq!(s.delete_chat("chat-1").unwrap(), 1);
    }
}
