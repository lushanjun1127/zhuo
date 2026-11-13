//! 后台记忆提取：N 轮未处理对话 → 1B 小模型总结 → 写回记忆库 + 标记已处理。
//!
//! 按「模型会碎」设计（蓝图 Phase 3）：
//! - 不强求 JSON 结构化——解析层尽力而为，拿到大概的总结文本就存；
//! - 任何一步失败都只跳过本轮（下轮对话会再触发），**绝不 panic**；
//! - 锁全部用 `unwrap_or_else(|e| e.into_inner())`，锁中毒也恢复而不是崩。

use anyhow::Result;
use zhuo_llm::{Msg, StreamConfig};

use crate::state::ServerState;

const EXTRACT_SYSTEM: &str = "你是对话记忆压缩器。你会收到一段角色扮演对话的记录，把它提炼成 1~2 句第三人称中文事实陈述，只保留持久事实（喜好、身份、关系、发生的事件），忽略寒暄与一次性问答。只输出陈述文本，不要续写对话。";

/// 组装提取 prompt。要点：
/// 1. 不用 `[user]/[assistant]` 这类形似 chat 模板的前缀——0.5B 小模型会把它当对话续写；
/// 2. 指令放在 transcript **之后**——小模型对结尾 token 的注意力更强。
fn extract_prompt(transcript: &str) -> Vec<Msg> {
    let lines = transcript
        .lines()
        .map(|l| {
            if let Some(r) = l.strip_prefix("[user] ") {
                format!("用户说：{r}")
            } else if let Some(r) = l.strip_prefix("[assistant] ") {
                format!("角色说：{r}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let user =
        format!("对话记录：\n{lines}\n\n请把以上对话提炼成 1~2 句第三人称事实陈述，直接输出：");
    vec![Msg::system(EXTRACT_SYSTEM), Msg::user(user)]
}

/// 从碎 JSON 里收割指定 key 的字符串值。
/// 只要 `"<key>"` 后的第一个完整引号字符串还在，即使外层结构碎了也能拿到。
fn harvest_quoted(raw: &str, keys: &[&str]) -> Option<String> {
    for k in keys {
        let pat = format!("\"{}\"", k);
        let Some(pos) = raw.find(&pat) else { continue };
        let rest = &raw[pos + pat.len()..];
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let after = rest[colon + 1..].trim_start();
        let Some(stripped) = after.strip_prefix('"') else {
            continue;
        };
        if let Some(end) = stripped.find('"') {
            let v = stripped[..end].trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 容错解析模型输出。优先级：
/// 1. 完整 JSON 的 `summary`/`总结` 字段；
/// 2. 碎 JSON 的引号收割；
/// 3. 剥离 markdown 围栏与 `总结：` 类标签前缀后的正文；
/// 4. 纯文本兜底（截断 200 字）。
/// 返回 None = 解析失败（太短或纯 JSON 垃圾），调用方应跳过本轮。
pub fn extract_summary(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }

    // qwen3 等思维链模型：剥掉 <think>…</think>，只取其后的正文；
    // 只有 <think> 没有闭合 = 全是推理没给答案 → 跳过
    let t = if let Some(pos) = t.rfind("</think>") {
        t[pos + "</think>".len()..].trim()
    } else if t.starts_with("<think>") {
        return None;
    } else {
        t
    };
    if t.is_empty() {
        return None;
    }

    // 1) 完整 JSON
    if let (Some(s), Some(e)) = (t.find('{'), t.rfind('}')) {
        if s < e {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t[s..=e]) {
                if let Some(txt) = v
                    .get("summary")
                    .or_else(|| v.get("总结"))
                    .and_then(|x| x.as_str())
                {
                    let txt = txt.trim();
                    if txt.chars().count() >= 4 {
                        return Some(txt.to_string());
                    }
                }
            }
        }
    }

    // 2) 碎 JSON 引号收割
    if let Some(v) = harvest_quoted(t, &["summary", "总结", "摘要"]) {
        if v.chars().count() >= 4 {
            return Some(v);
        }
    }

    // 3) 剥围栏行 + 标签前缀
    let mut lines: Vec<&str> = Vec::new();
    for l in t.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with("```") {
            continue;
        }
        lines.push(l);
    }
    let mut text = lines.join(" ");
    for pre in [
        "总结：",
        "总结:",
        "摘要：",
        "摘要:",
        "记忆：",
        "记忆:",
        "Summary:",
    ] {
        if let Some(stripped) = text.strip_prefix(pre) {
            text = stripped.trim().to_string();
            break;
        }
    }
    let text = text
        .trim_matches(|c| matches!(c, '"' | '“' | '”' | '「' | '」' | '『' | '』'))
        .trim()
        .to_string();

    // 看起来像 JSON 但没收割成功 → 视为解析失败，跳过
    if text.starts_with('{') || text.starts_with('[') {
        return None;
    }
    let text: String = text.chars().take(200).collect();
    if text.chars().count() < 4 {
        return None;
    }
    Some(text)
}

/// 执行一次提取。返回 Ok(Some(summary)) 表示成功写入；
/// Ok(None) 表示本轮跳过（消息不足 / 模型输出不可解析）；Err 只在最底层 IO 故障时出现。
pub async fn run_extraction(st: &ServerState, card: &str) -> Result<Option<String>> {
    let connection = crate::routes::connection_settings(st);
    let base_url = crate::routes::api_base_url(&connection, st.ollama_native);
    if connection.summary_model.trim().is_empty() {
        return Ok(None);
    }

    // 1) 快照未处理对话（短临界区，不跨 await 持锁）
    let (transcript, upto_id) = {
        let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
        let count = mem.count_unprocessed(card)?;
        if count < st.summary_rounds {
            return Ok(None); // 还不够一轮批量，等下次触发
        }
        let rows = mem.unprocessed(card, 60)?;
        let upto_id = rows.last().map(|(id, _)| *id).unwrap_or(0);
        let transcript = rows
            .iter()
            .map(|(_, m)| format!("[{}] {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        (transcript, upto_id)
    };
    if transcript.trim().is_empty() {
        return Ok(None);
    }

    // 2) 调提取模型（常驻 1B 通道；temperature=0 可复现；不强求 JSON——容错解析兜底）
    let msgs = extract_prompt(&transcript);
    let cfg = StreamConfig {
        base_url: base_url.as_str(),
        api_key: connection.api_key.as_str(),
        model: connection.summary_model.as_str(),
        include_usage: false,
        dump_path: None,
        response_format: None,
        temperature: Some(0.0),
        max_tokens: None,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        repetition_penalty: None,
        // 提取模型强制走 CPU，避免小模型抢占主模型显存
        num_gpu: Some(0),
        ollama_native: st.ollama_native,
    };
    let outcome = zhuo_llm::stream_chat_ex(&cfg, &msgs, |_| true).await?;

    // 3) 容错解析；失败直接跳过（不写记忆、不标记——下轮对话会再次触发）
    let Some(summary) = extract_summary(&outcome.content) else {
        let head: String = outcome.content.chars().take(80).collect();
        eprintln!("[summarize] {card}: 输出不可解析，本轮跳过: {head:?}");
        return Ok(None);
    };

    // 4) 写回记忆库 + 标记已处理
    {
        let mem = st.memory.lock().unwrap_or_else(|e| e.into_inner());
        mem.add_memory(card, &summary, "", upto_id)?;
        mem.mark_processed(card, upto_id)?;
    }
    Ok(Some(summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: Option<String>) -> String {
        x.unwrap()
    }

    #[test]
    fn valid_json_summary() {
        let raw = r#"{"summary": "苏菲在集市买了一串山楂糖葫芦。"}"#;
        assert_eq!(s(extract_summary(raw)), "苏菲在集市买了一串山楂糖葫芦。");
        // 中文 key
        assert_eq!(
            s(extract_summary("{\"总结\": \"冬堡常年下雪。\"}")),
            "冬堡常年下雪。"
        );
    }

    #[test]
    fn broken_json_still_harvests_string() {
        // 碎 JSON：缺结尾大括号
        let raw = "{\"summary\": \"苏菲喜欢在冬天吃糖葫芦\"";
        assert_eq!(s(extract_summary(raw)), "苏菲喜欢在冬天吃糖葫芦");
        // 碎 JSON：多余文字混入
        let raw = "好的，总结如下：{\"总结\": \"苏菲的猫叫雪球\"} 以上。";
        assert_eq!(s(extract_summary(raw)), "苏菲的猫叫雪球");
    }

    #[test]
    fn fenced_plain_text() {
        let raw = "```text\n苏菲今天在图书馆借了一本禁书。\n```";
        assert_eq!(s(extract_summary(raw)), "苏菲今天在图书馆借了一本禁书。");
    }

    #[test]
    fn label_prefix_stripped() {
        assert_eq!(s(extract_summary("总结：苏菲爱吃甜食。")), "苏菲爱吃甜食。");
        assert_eq!(
            s(extract_summary("Summary: Sophie likes sweets.")),
            "Sophie likes sweets."
        );
        // 引号包裹
        assert_eq!(s(extract_summary("“苏菲养了一只猫。”")), "苏菲养了一只猫。");
    }

    #[test]
    fn plain_text_passthrough_and_clamp() {
        assert_eq!(
            s(extract_summary("苏菲今天心情很好。")),
            "苏菲今天心情很好。"
        );
        let long = "长".repeat(300);
        let out = extract_summary(&long).unwrap();
        assert_eq!(out.chars().count(), 200);
    }

    #[test]
    fn garbage_is_skipped() {
        assert!(extract_summary("").is_none());
        assert!(extract_summary("   \n  ").is_none());
        // 太短
        assert!(extract_summary("嗯").is_none());
        // 像 JSON 但收不到任何字符串 → 跳过
        assert!(extract_summary("{\"summary\":").is_none());
        assert!(extract_summary("{ broken }").is_none());
    }

    #[test]
    fn think_block_stripped_for_reasoning_models() {
        // qwen3 系：正文在 </think> 之后
        let raw =
            "<think>用户在问猫，我需要从对话里找猫的信息……</think>\n苏菲的猫叫雪球，只吃蜂蜜。";
        assert_eq!(s(extract_summary(raw)), "苏菲的猫叫雪球，只吃蜂蜜。");
        // think 里带 JSON 干扰也不影响
        let raw = "<think>{\"fake\": \"x\"}</think>苏菲今天去了集市。";
        assert_eq!(s(extract_summary(raw)), "苏菲今天去了集市。");
        // 只有 <think> 没闭合 → 跳过
        assert!(extract_summary("<think>我想想……").is_none());
    }
}
