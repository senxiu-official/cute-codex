use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn page(bytes: &[u8]) -> Value {
    json!({"jobId":"j", "stream":"stdout", "fromOffset":7,
        "nextOffset":7 + bytes.len(), "bytesHex":bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "gap":false,"truncated":false})
}
fn parsed(value: &Value) -> Option<OutputPage> {
    OutputPage::parse(value, &json!({"jobId":"j","stream":"stdout"}))
}
#[test]
fn output_page_plain_unicode_and_source_flags() {
    let mut value = page("**plain** https://example.invalid\n🦀e\u{301}\u{1b}[31m".as_bytes());
    value["gap"] = json!(true);
    value["truncated"] = json!(true);
    let lines = parsed(&value).unwrap().lines(28);
    insta::assert_snapshot!(
        lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!lines.iter().any(|line| line.to_string().contains('\u{1b}')));
}
#[test]
fn output_page_validates_exact_bytes_and_offsets() {
    for (key, replacement) in [
        ("bytesHex", json!("0x")),
        ("bytesHex", json!("f")),
        ("bytesHex", json!("é")),
        ("nextOffset", json!(99)),
        ("gap", json!(null)),
        ("stream", json!("other")),
    ] {
        let mut value = page(b"abc");
        value[key] = replacement;
        assert!(parsed(&value).is_none(), "{key}");
    }
    assert_eq!(parsed(&page(b"abc")).unwrap().bytes, b"abc");
}
#[test]
fn output_page_binary_empty_and_preview_window() {
    let binary = parsed(&page(&[0xff, 0xfe])).unwrap().lines(80);
    assert!(
        binary
            .iter()
            .any(|line| line.to_string().contains("Binary output · 2 B"))
    );
    let empty = parsed(&page(b"")).unwrap().lines(80);
    assert!(empty[0].to_string().contains("empty page"));
    for text in [
        "a\n".repeat(20),
        "🦀".repeat(1024),
        format!("{}e\u{301}", "x".repeat(2047)),
    ] {
        let lines = parsed(&page(text.as_bytes())).unwrap().lines(80);
        assert!(
            lines
                .iter()
                .any(|line| line.to_string().contains("Preview shortened"))
        );
        assert!(lines.len() <= 8);
    }
}

#[test]
fn output_page_exact_byte_cap_and_grapheme_boundary() {
    let exact = parsed(&page("x".repeat(2048).as_bytes()))
        .unwrap()
        .lines(4096);
    assert_eq!(exact.len(), 2);
    assert_eq!(exact[1].to_string(), format!("  {}", "x".repeat(2048)));
    let over = parsed(&page("x".repeat(2049).as_bytes()))
        .unwrap()
        .lines(4096);
    assert_eq!(over[1], exact[1]);
    assert!(over[2].to_string().contains("Preview shortened"));
    let combined = format!("{}e\u{301}", "x".repeat(2047));
    let lines = parsed(&page(combined.as_bytes())).unwrap().lines(4096);
    assert_eq!(lines[1].to_string(), format!("  {}", "x".repeat(2047)));
    assert!(lines[2].to_string().contains("Preview shortened"));
    let mut overflow = page(b"x");
    overflow["fromOffset"] = json!(u64::MAX);
    overflow["nextOffset"] = json!(0);
    assert!(parsed(&overflow).is_none());
}

#[test]
fn output_page_initial_and_later_metadata() {
    let mut initial = page(&[b'x'; 39]);
    initial["fromOffset"] = json!(0);
    initial["nextOffset"] = json!(39);
    assert_eq!(
        parsed(&initial).unwrap().lines(80)[0].to_string(),
        "  stdout · 39 B"
    );
    let mut empty = page(b"");
    empty["fromOffset"] = json!(0);
    empty["nextOffset"] = json!(0);
    let cases = [initial, page(b"abc"), empty, page(b"")];
    insta::assert_snapshot!(
        cases
            .iter()
            .map(|value| parsed(value).unwrap().lines(80)[0].to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn output_page_ordinary_preview_is_dim() {
    use ratatui::style::Modifier;
    let value = page("plain\n世界 e\u{301}".as_bytes());
    let original = value.clone();
    let output = parsed(&value).unwrap();
    let lines = output.lines(12);
    assert!(
        lines
            .iter()
            .all(|line| line.style.add_modifier.contains(Modifier::DIM))
    );
    assert_eq!(output.bytes, "plain\n世界 e\u{301}".as_bytes());
    assert_eq!(value, original);
}

#[test]
fn service_r2_text_pages_use_raw_offsets_even_for_lossy_utf8() {
    for (text, bytes) in [("hello", 5), ("中文", 6), ("�", 1), ("", 0)] {
        let value = json!({"jobId":"j", "stream":"stdout", "fromOffset":7,
            "nextOffset":7 + bytes, "text":text, "encoding":"utf-8-lossy",
            "gap":true, "truncated":false, "pageLimited":true});
        let parsed = parsed(&value).unwrap();
        let lines: Vec<String> = parsed.lines(80).iter().map(ToString::to_string).collect();
        assert!(lines[0].contains(&format!("{bytes} B")));
        assert!(lines.iter().any(|line| line.contains("Page limited")));
        assert!(lines.iter().any(|line| line.contains("Output gap")));
        if !text.is_empty() {
            assert!(lines.iter().any(|line| line.contains(text)));
        }
    }
}
