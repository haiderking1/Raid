use super::render;
use ratatui::text::Line;

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn renders_emphasis_and_inline_code() {
    let lines = render("hello **bold** and `code`", 40);
    let joined = text(&lines).join("\n");
    assert!(joined.contains("hello"));
    assert!(joined.contains("bold"));
    assert!(joined.contains("code"));
    assert!(lines.iter().flat_map(|line| &line.spans).any(|span| {
        span.content.as_ref() == "bold"
            && span
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
            || span
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
                && span.content.as_ref().contains("bold")
    }));
}

#[test]
fn renders_headings_lists_quotes_and_fences() {
    let source = "# Title\n\n- one\n- two\n\n> quoted\n\n```rs\nfn main() {}\n```\n";
    let lines = render(source, 40);
    let joined = text(&lines).join("\n");
    assert!(joined.contains("# Title"));
    assert!(joined.contains("• one"));
    assert!(joined.contains("• two"));
    assert!(joined.contains("▌ quoted"));
    assert!(joined.contains("  rs") || joined.contains("│ fn main() {}"));
    assert!(joined.contains("│ fn main() {}"));
}

#[test]
fn wraps_paragraphs_to_the_content_width() {
    let lines = render("alpha beta gamma delta", 10);
    assert!(lines.len() >= 2);
    for line in &lines {
        assert!(
            line.width() <= 10,
            "{}",
            text(std::slice::from_ref(line)).join("")
        );
    }
}

#[test]
fn paragraphs_keep_spaces_between_words() {
    let joined = text(&render("hello world", 40)).join("\n");
    assert_eq!(joined, "hello world");
}

#[test]
fn links_keep_their_label() {
    let lines = render("see [docs](https://example.com)", 40);
    let joined = text(&lines).join("\n");
    assert!(joined.contains("docs"));
}

#[test]
fn task_items_keep_the_checkbox_on_the_bullet() {
    let lines = render("- [x] shipped\n- [ ] later", 40);
    let joined = text(&lines).join("\n");
    assert!(joined.contains("• [x] shipped"));
    assert!(joined.contains("• [ ] later"));
}

#[test]
fn wrapped_list_items_keep_a_hanging_indent() {
    let lines = render("- hello world this wraps", 12);
    let rendered = text(&lines);
    assert!(rendered[0].starts_with("• hello"));
    assert!(
        rendered
            .iter()
            .any(|line| line.starts_with("  ") && line.contains("wraps"))
    );
    for line in &lines {
        assert!(
            line.width() <= 12,
            "{}",
            text(std::slice::from_ref(line)).join("")
        );
    }
}

#[test]
fn long_list_tokens_wrap_inside_the_content_width() {
    let lines = render("- src/main.rs", 12);
    let compact: String = text(&lines)
        .join("")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(compact.contains("src/main.rs"));
    for line in &lines {
        assert!(
            line.width() <= 12,
            "{}",
            text(std::slice::from_ref(line)).join("")
        );
    }
}

#[test]
fn quoted_list_wraps_without_doubling_the_gutter() {
    let lines = render("> - hello world this wraps", 16);
    let rendered = text(&lines);
    assert!(rendered[0].starts_with("▌ • hello"));
    for line in &rendered {
        assert_eq!(line.matches('▌').count(), 1, "{line}");
    }
    assert!(
        rendered
            .iter()
            .any(|line| line.starts_with("▌   ") && line.contains("wraps"))
    );
    for line in &lines {
        assert!(
            line.width() <= 16,
            "{}",
            text(std::slice::from_ref(line)).join("")
        );
    }
}

#[test]
fn language_less_fence_inside_a_list_keeps_the_bullet() {
    let lines = render("- ```\n  fn x() {}\n  ```\n", 40);
    let joined = text(&lines).join("\n");
    assert!(joined.contains('•'), "{joined:?}");
    assert!(joined.contains("│ fn x() {}"), "{joined:?}");
}
