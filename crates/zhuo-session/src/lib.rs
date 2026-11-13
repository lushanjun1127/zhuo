//! 会话历史落盘：CLI 与 Web 共用。
//! 每个角色一个 JSON 文件，只存对话轮次；system 每次由卡重建。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zhuo_llm::Msg;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Session {
    pub card_name: String,
    pub messages: Vec<Msg>,
}

/// 会话目录：优先 ZHUO_SESSIONS_DIR，否则 ~/.zhuo/sessions
pub fn sessions_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ZHUO_SESSIONS_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".zhuo").join("sessions")
}

/// 角色名转安全文件名（保留中文/字母数字，其余替换为 _）
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "unnamed".into()
    } else {
        s
    }
}

impl Session {
    pub fn path(card_name: &str, dir: &Path) -> PathBuf {
        dir.join(format!("{}.json", safe_name(card_name)))
    }

    /// 读已有历史；文件不存在或损坏时返回空会话（不阻塞启动）
    pub fn load_or_new(card_name: &str, dir: &Path) -> Result<Self> {
        let path = Self::path(card_name, dir);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parse session: {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                card_name: card_name.into(),
                messages: Vec::new(),
            }),
            Err(e) => Err(e).with_context(|| format!("read session: {}", path.display())),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = Self::path(&self.card_name, dir);
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("write session: {}", path.display()))?;
        Ok(())
    }

    /// 清空会话时同步删盘上文件，避免重启后"幽灵恢复"
    pub fn delete(card_name: &str, dir: &Path) -> Result<()> {
        let path = Self::path(card_name, dir);
        match std::fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("delete session: {}", path.display())),
        }
    }

    /// 启动时扫目录恢复全部会话；跳过损坏或非 Session 的 .json
    pub fn load_all(dir: &Path) -> Vec<Self> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&p) {
                if let Ok(s) = serde_json::from_slice::<Session>(&bytes) {
                    if !s.card_name.is_empty() {
                        out.push(s);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.card_name.cmp(&b.card_name));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "zhuo_shared_session_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tmp("roundtrip");
        let mut s = Session::load_or_new("苏菲/测试", &dir).unwrap();
        assert!(s.messages.is_empty());
        s.messages.push(Msg::user("你好"));
        s.messages.push(Msg::assistant("要来串冰糖葫芦吗？"));
        s.save(&dir).unwrap();

        let loaded = Session::load_or_new("苏菲/测试", &dir).unwrap();
        assert_eq!(loaded, s);
        assert_eq!(loaded.messages.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_gives_empty_session() {
        let dir = tmp("missing");
        let s = Session::load_or_new("nobody", &dir).unwrap();
        assert_eq!(s.card_name, "nobody");
        assert!(s.messages.is_empty());
    }

    #[test]
    fn safe_name_replaces_path_separators() {
        assert_eq!(safe_name("苏菲/测试"), "苏菲_测试");
        assert_eq!(safe_name("a:b\\c"), "a_b_c");
        assert_eq!(safe_name("///"), "___");
    }

    #[test]
    fn load_all_skips_corrupt_and_non_session_files() {
        let dir = tmp("load_all");
        std::fs::create_dir_all(&dir).unwrap();
        Session {
            card_name: "苏菲".into(),
            messages: vec![Msg::user("一")],
        }
        .save(&dir)
        .unwrap();
        Session {
            card_name: "冬堡".into(),
            messages: vec![Msg::user("二")],
        }
        .save(&dir)
        .unwrap();
        std::fs::write(dir.join("broken.json"), b"not json").unwrap();
        std::fs::write(dir.join("x.tokens.jsonl"), b"{}\n").unwrap();

        let all = Session::load_all(&dir);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].card_name, "冬堡");
        assert_eq!(all[1].card_name, "苏菲");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_file() {
        let dir = tmp("delete");
        let s = Session {
            card_name: "苏菲".into(),
            messages: vec![Msg::user("一")],
        };
        s.save(&dir).unwrap();
        assert!(Session::path("苏菲", &dir).exists());
        Session::delete("苏菲", &dir).unwrap();
        assert!(!Session::path("苏菲", &dir).exists());
        Session::delete("苏菲", &dir).unwrap(); // 幂等
        let _ = std::fs::remove_dir_all(&dir);
    }
}
