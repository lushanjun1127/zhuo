//! 声明式 Prompt 组装：system = 角色描述 + 示例对话 + 世界设定；
//! 记忆块作为**独立 system 消息**插在主 system 与历史之间（蓝图 3.1 注入机制）。

use serde_json::Value;
use zhuo_card::Card;
use zhuo_llm::Msg;

pub fn system_prompt(card: &Card, world: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !card.system_prompt.trim().is_empty() {
        parts.push(card.system_prompt.trim().to_string());
    }
    if !card.description.trim().is_empty() {
        parts.push(card.description.trim().to_string());
    }
    if !card.personality.trim().is_empty() {
        parts.push(format!("[Personality]\n{}", card.personality.trim()));
    }
    if !card.scenario.trim().is_empty() {
        parts.push(format!("[Scenario]\n{}", card.scenario.trim()));
    }
    if !card.mes_example.trim().is_empty() {
        parts.push(card.mes_example.trim().to_string());
    }
    if !world.trim().is_empty() {
        parts.push(format!("[世界设定]\n{}", world.trim()));
    }
    parts.join("\n\n")
}

/// 世界设定 JSON 全量拍平：递归收集所有字符串值。
pub fn world_text(v: &Value) -> String {
    let mut parts = Vec::new();
    collect(v, &mut parts);
    parts.join("\n\n")
}

fn collect(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if !s.trim().is_empty() {
                out.push(s.trim().to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect(item, out);
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                match val {
                    Value::String(s) if !s.trim().is_empty() => {
                        out.push(format!("[{}]\n{}", key, s.trim()));
                    }
                    Value::String(_) => {}
                    _ => collect(val, out),
                }
            }
        }
        _ => {}
    }
}

/// [system] + 历史 + [user]；超长时从最早的非 system 消息开始丢，直到放得下。
pub fn build_messages(system: &str, history: &[Msg], input: &str, max_chars: usize) -> Vec<Msg> {
    let mut msgs = Vec::with_capacity(history.len() + 2);
    msgs.push(Msg::system(system));
    msgs.extend(history.iter().cloned());
    msgs.push(Msg::user(input));

    while msgs.len() > 2 && total_chars(&msgs) > max_chars {
        msgs.remove(1);
    }
    msgs
}

/// 最终组装：主 system → [记忆块（独立 system 消息，本轮生成后即丢弃）] → 历史 → 当前输入。
pub fn assemble_messages(
    system: &str,
    memory_block: Option<&str>,
    history: &[Msg],
    input: &str,
    max_chars: usize,
) -> Vec<Msg> {
    let mut msgs = build_messages(system, history, input, max_chars);
    if let Some(block) = memory_block {
        if !block.trim().is_empty() {
            msgs.insert(1, Msg::system(block));
        }
    }
    msgs
}

/// 渲染 [当前状态] 块（蓝图 5.2）：顶层键值各占一行，数组用「、」连接；
/// 空对象/无有效字段返回 None（prompt 不被空状态污染）。嵌套对象暂不渲染。
pub fn render_state(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    let mut lines: Vec<String> = Vec::new();
    for (k, val) in obj {
        match val {
            Value::String(s) if !s.trim().is_empty() => {
                lines.push(format!("{k}：{}", s.trim()));
            }
            Value::Array(a) if !a.is_empty() => {
                let items = a
                    .iter()
                    .map(|x| match x {
                        Value::String(s) => s.trim().to_string(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("、");
                lines.push(format!("{k}：{items}"));
            }
            Value::Number(n) => lines.push(format!("{k}：{n}")),
            Value::Bool(b) => lines.push(format!("{k}：{b}")),
            _ => {}
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("[当前状态]\n{}", lines.join("\n")))
}

fn total_chars(msgs: &[Msg]) -> usize {
    msgs.iter().map(|m| m.content.chars().count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card {
            id: "deadbeef".into(),
            name: "苏菲".into(),
            description: "冬堡学院的学生".into(),
            first_mes: "要来串糖葫芦吗？".into(),
            mes_example: "<START>".into(),
            ..Default::default()
        }
    }

    #[test]
    fn system_prompt_contains_card_and_world() {
        let s = system_prompt(&card(), "冰糖葫芦是冬堡特产");
        assert!(s.contains("冬堡学院的学生"));
        assert!(s.contains("<START>"));
        assert!(s.contains("[世界设定]"));
        assert!(s.contains("冰糖葫芦是冬堡特产"));
    }

    #[test]
    fn world_text_flattens_nested_json() {
        let v = serde_json::json!({
            "冬堡": "常年下雪",
            "entries": [{"content": "学院在山巅"}, {"content": "糖葫芦很甜"}],
            "count": 3
        });
        let t = world_text(&v);
        assert!(t.contains("常年下雪"));
        assert!(t.contains("学院在山巅"));
        assert!(t.contains("糖葫芦很甜"));
    }

    #[test]
    fn truncation_drops_oldest_non_system() {
        let history = vec![
            Msg::user("旧问题"),
            Msg::assistant("旧回答"),
            Msg::user("较新的问题"),
            Msg::assistant("较新的回答"),
        ];
        let msgs = build_messages("SYS", &history, "新问题", 14);
        assert_eq!(msgs.first().unwrap().role, "system");
        assert_eq!(msgs.last().unwrap().content, "新问题");
        assert!(!msgs.iter().any(|m| m.content == "旧问题"));
    }

    #[test]
    fn newest_user_message_is_never_dropped() {
        let history: Vec<Msg> = (0..50)
            .map(|i| Msg::user(format!("历史消息{i:020}")))
            .collect();
        let msgs = build_messages("SYS", &history, "当前输入", 10);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].content, "当前输入");
    }

    #[test]
    fn memory_block_sits_between_system_and_history() {
        let history = vec![Msg::user("上一问"), Msg::assistant("上一答")];
        let msgs = assemble_messages(
            "SYS",
            Some("[相关记忆]\n- 苏菲爱吃糖葫芦"),
            &history,
            "新问题",
            12000,
        );
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.starts_with("SYS"));
        assert_eq!(msgs[1].role, "system");
        assert!(msgs[1].content.contains("相关记忆"));
        assert_eq!(msgs[2].content, "上一问");
        assert_eq!(msgs.last().unwrap().content, "新问题");
        // 无记忆时不插空 system
        let msgs = assemble_messages("SYS", None, &history, "新问题", 12000);
        assert_eq!(msgs[1].content, "上一问");
    }

    #[test]
    fn render_state_formats_arrays_strings_numbers() {
        let v = serde_json::json!({"背包": ["木剑", "药水"], "位置": "冬堡学院", "好感度": 5});
        let s = render_state(&v).unwrap();
        assert!(s.starts_with("[当前状态]"));
        assert!(s.contains("背包：木剑、药水"));
        assert!(s.contains("位置：冬堡学院"));
        assert!(s.contains("好感度：5"));
        // 空对象 → None
        assert!(render_state(&serde_json::json!({})).is_none());
        // 全空值 → None
        assert!(render_state(&serde_json::json!({"x": "", "y": []})).is_none());
    }

    #[test]
    fn system_prompt_appends_state_block() {
        let mut sys = system_prompt(&card(), "冬堡特产糖葫芦");
        if let Some(sb) = render_state(&serde_json::json!({"位置": "冬堡学院"})) {
            sys.push_str("\n\n");
            sys.push_str(&sb);
        }
        assert!(sys.contains("[世界设定]"));
        assert!(sys.contains("[当前状态]"));
        assert!(sys.contains("位置：冬堡学院"));
    }
}
