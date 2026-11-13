//! 由真实/模拟 SSE dump 裁出的金样，逐项固化 SseParser 行为。
//! 覆盖：event: 行、: ping 注释、无 [DONE]、错误帧、多行 data、中文多字节分包、usage 帧。
//! `ollama_real_*.sse` 来自本地 Ollama 真模型（非 mock），含 choices:[] 的 usage 帧与 timings。

use zhuo_llm::{SseEvent, SseParser, Usage};

const MIXED: &[u8] = include_bytes!("fixtures/mixed.sse");
const NO_DONE: &[u8] = include_bytes!("fixtures/no_done.sse");
const ERROR: &[u8] = include_bytes!("fixtures/error.sse");
const MULTILINE: &[u8] = include_bytes!("fixtures/multiline_data.sse");
/// 3 轮中文基线跑出来的真实原始字节流（未经任何修饰）
const BASELINE_TURN1: &[u8] = include_bytes!("fixtures/baseline_turn1.sse");
/// 本地 Ollama 真模型 3 轮对话的第 1 / 第 3 轮原始 dump
const OLLAMA_TURN1: &[u8] = include_bytes!("fixtures/ollama_real_turn1.sse");
const OLLAMA_TURN3: &[u8] = include_bytes!("fixtures/ollama_real_turn3.sse");

fn tokens(events: &[SseEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            SseEvent::Token(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn prompt_tokens(events: &[SseEvent]) -> Option<u64> {
    events.iter().find_map(|e| match e {
        SseEvent::Usage(u) => u.prompt_tokens,
        _ => None,
    })
}

fn usage(events: &[SseEvent]) -> Option<Usage> {
    events.iter().find_map(|e| match e {
        SseEvent::Usage(u) => Some(u.clone()),
        _ => None,
    })
}

fn feed_all(bytes: &[u8], chunk: usize) -> (SseParser, Vec<SseEvent>) {
    let mut parser = SseParser::new();
    let mut all = Vec::new();
    for part in bytes.chunks(chunk) {
        all.extend(parser.push(part).unwrap());
        if parser.done {
            break;
        }
    }
    all.extend(parser.finish().unwrap());
    (parser, all)
}

#[test]
fn mixed_dump_event_lines_comments_usage_and_done() {
    let (parser, events) = feed_all(MIXED, MIXED.len());
    assert_eq!(tokens(&events), "你好，世界");
    assert_eq!(prompt_tokens(&events), Some(128));
    assert!(parser.done, "应识别 data: [DONE]");
    assert_eq!(events.last(), Some(&SseEvent::Done));
}

#[test]
fn mixed_dump_survives_byte_by_byte_multibyte_split() {
    // 逐字节喂入：中文 UTF-8 三字节码必然被切开
    let (parser, events) = feed_all(MIXED, 1);
    assert_eq!(tokens(&events), "你好，世界");
    assert_eq!(prompt_tokens(&events), Some(128));
    assert!(parser.done);
}

#[test]
fn mixed_dump_survives_odd_chunk_sizes() {
    for size in [2, 3, 5, 7, 13, 31, 64] {
        let (_, events) = feed_all(MIXED, size);
        assert_eq!(tokens(&events), "你好，世界", "chunk size {size}");
        assert_eq!(prompt_tokens(&events), Some(128), "chunk size {size}");
    }
}

#[test]
fn no_done_dump_completes_at_eof() {
    let (parser, events) = feed_all(NO_DONE, NO_DONE.len());
    assert_eq!(tokens(&events), "屋檐上的雪很甜");
    assert!(!parser.done, "无 [DONE] 时 done 应为 false");
}

#[test]
fn no_done_dump_also_works_byte_by_byte() {
    let (_, events) = feed_all(NO_DONE, 1);
    assert_eq!(tokens(&events), "屋檐上的雪很甜");
}

#[test]
fn error_frame_returns_err() {
    let mut parser = SseParser::new();
    let err = parser.push(ERROR).unwrap_err();
    assert!(err.to_string().contains("rate_limit_exceeded"));
}

#[test]
fn multiline_data_lines_join_into_one_event() {
    let (parser, events) = feed_all(MULTILINE, 3);
    assert_eq!(tokens(&events), "糖葫芦");
    assert!(parser.done);
}

#[test]
fn captured_baseline_dump_replays_exactly() {
    // 5 字节切片写出的真实 dump：中文被切断过，逐字节回放仍须完整
    for size in [1, 5, 17, BASELINE_TURN1.len()] {
        let (parser, events) = feed_all(BASELINE_TURN1, size);
        let text = tokens(&events);
        assert!(text.starts_with("收到第1轮"), "chunk {size}: {text}");
        assert!(
            text.contains("糖葫芦的糖壳还是脆的"),
            "chunk {size}: {text}"
        );
        assert_eq!(prompt_tokens(&events), Some(135), "chunk {size}");
        assert!(parser.done, "chunk {size}");
    }
}

#[test]
fn real_ollama_dump_parses_tokens_and_usage() {
    // 真模型帧：首帧 delta 含 role、末帧 delta:{} + finish_reason、
    // usage 帧 choices:[] 与 timings，最后 [DONE]
    let (parser, events) = feed_all(OLLAMA_TURN1, OLLAMA_TURN1.len());
    let text = tokens(&events);
    assert!(text.starts_with("下雪了"), "{text}");
    assert!(text.contains("糖葫芦是我最喜欢吃的"), "{text}");
    assert!(text.ends_with("非常受欢迎。"), "{text}");
    let u = usage(&events).expect("usage frame");
    assert_eq!(u.prompt_tokens, Some(193));
    assert_eq!(u.completion_tokens, Some(73));
    assert_eq!(u.total_tokens, Some(266));
    assert!(parser.done);
}

#[test]
fn real_ollama_dump_survives_byte_by_byte() {
    for size in [1, 3, 7, 64] {
        let (parser, events) = feed_all(OLLAMA_TURN3, size);
        let text = tokens(&events);
        assert!(text.contains("屋檐下的白雪"), "chunk {size}: {text}");
        assert!(text.ends_with("甜到心。"), "chunk {size}: {text}");
        assert_eq!(prompt_tokens(&events), Some(346), "chunk {size}");
        assert!(parser.done, "chunk {size}");
    }
}
