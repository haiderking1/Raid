use super::status::{ToolCall, ToolStatus};
use crate::frontend::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
};
use unicode_segmentation::UnicodeSegmentation;

const TOOL: Color = Color::Rgb(70, 183, 128);
const PATH: Color = Color::Rgb(139, 180, 250);
const ARG: Color = Color::Rgb(220, 220, 220);
const PUNCT: Color = Color::Rgb(160, 160, 160);
const RESULT: Color = Color::Rgb(196, 196, 196);
const HINT: Color = Color::Rgb(120, 120, 120);
const FAILED: Color = Color::Rgb(212, 176, 120);
const PREFIX: &str = "● ";
const RESULT_PREFIX: &str = "  └ ";

pub(crate) fn paint_header(buf: &mut Buffer, area: Rect, y: u16, call: &ToolCall) {
    let mut x = area.x;
    let end = area.x.saturating_add(area.width);
    let name = call.display_name();
    paint_span(buf, &mut x, end, y, PREFIX, tool_style());
    paint_span(buf, &mut x, end, y, &name, tool_style());
    paint_span(buf, &mut x, end, y, "(", Style::default().fg(PUNCT));
    let available = end.saturating_sub(x) as usize;
    let detail_width = available.saturating_sub(1);
    let detail = clip_right(&call.detail, detail_width);
    paint_span(
        buf,
        &mut x,
        end,
        y,
        &detail,
        Style::default().fg(arg_color(&call.detail)),
    );
    paint_span(buf, &mut x, end, y, ")", Style::default().fg(PUNCT));
}

pub(crate) fn paint_result(buf: &mut Buffer, area: Rect, y: u16, call: &ToolCall) {
    let mut x = area.x;
    let end = area.x.saturating_add(area.width);
    paint_span(
        buf,
        &mut x,
        end,
        y,
        RESULT_PREFIX,
        Style::default().fg(HINT),
    );
    let style = result_style(call.status);
    let summary = call.compact_summary();
    if let Some((body, hint)) = split_hint(&summary) {
        paint_span(buf, &mut x, end, y, body, style);
        paint_span(buf, &mut x, end, y, hint, Style::default().fg(HINT));
    } else {
        paint_span(buf, &mut x, end, y, &summary, style);
    }
}

pub(crate) fn paint_output_line(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    call: &ToolCall,
    line: &str,
    first: bool,
) {
    let mut x = area.x;
    let end = area.x.saturating_add(area.width);
    let prefix = if first { RESULT_PREFIX } else { "    " };
    paint_span(buf, &mut x, end, y, prefix, Style::default().fg(HINT));
    paint_span(buf, &mut x, end, y, line, result_style(call.status));
}

fn tool_style() -> Style {
    Style::default().fg(TOOL).add_modifier(Modifier::BOLD)
}

fn arg_color(detail: &str) -> Color {
    if detail.contains('/') || detail.contains('\\') {
        PATH
    } else {
        ARG
    }
}

fn result_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Failed => Style::default().fg(FAILED),
        ToolStatus::Running => Style::default().fg(TOOL),
        ToolStatus::Success => Style::default().fg(RESULT),
    }
}

fn split_hint(summary: &str) -> Option<(&str, &str)> {
    let index = summary.find(" (ctrl+")?;
    Some((&summary[..index], &summary[index..]))
}

fn clip_right(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if Line::from(text).width() <= width {
        return text.to_string();
    }
    if width == 1 {
        return String::from("…");
    }

    let mut clipped = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = Line::from(grapheme).width();
        if used + grapheme_width > width - 1 {
            break;
        }
        clipped.push_str(grapheme);
        used += grapheme_width;
    }
    clipped.push('…');
    clipped
}

fn paint_span(buf: &mut Buffer, x: &mut u16, end: u16, y: u16, text: &str, style: Style) {
    if *x >= end || text.is_empty() {
        return;
    }
    let remaining = (end.saturating_sub(*x)) as usize;
    let drawn = Line::from(text).width().min(remaining);
    render_clipped(buf, *x, y, text, remaining, style);
    *x = x.saturating_add(drawn as u16);
}

#[cfg(test)]
mod tests {
    use super::{PATH, TOOL, ToolCall};
    use crate::frontend::tools::ToolStatus;
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn success_card_uses_name_parens_and_result_branch() {
        let call = ToolCall::running("read", "src/raid/main.rs")
            .finished(ToolStatus::Success, "first\nsecond\nthird");
        let mut terminal = Terminal::new(TestBackend::new(48, 2)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::paint_header(frame.buffer_mut(), area, area.y, &call);
                super::paint_result(frame.buffer_mut(), area, area.y + 1, &call);
            })
            .unwrap();

        let rendered = screen(&terminal, 48, 2);
        assert!(rendered.contains("● Read(src/raid/main.rs)"));
        assert!(rendered.contains("└ Read 3 lines (ctrl+o to expand)"));
        assert!(!rendered.contains("✓"));
        assert!(!rendered.contains("done"));

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "●");
        assert_eq!(buffer.cell((0, 0)).unwrap().fg, TOOL);
        let path_x = ratatui::text::Line::from("● Read(").width() as u16;
        assert_eq!(buffer.cell((path_x, 0)).unwrap().symbol(), "s");
        assert_eq!(buffer.cell((path_x, 0)).unwrap().fg, PATH);
    }

    #[test]
    fn running_card_keeps_the_result_slot() {
        let call = ToolCall::running("bash", "cargo test --offline");
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::paint_header(frame.buffer_mut(), area, area.y, &call);
                super::paint_result(frame.buffer_mut(), area, area.y + 1, &call);
            })
            .unwrap();
        let rendered = screen(&terminal, 40, 2);
        assert!(rendered.contains("● Bash(cargo test --offline)"));
        assert!(rendered.contains("└ Running"));
        assert!(!rendered.contains("ctrl+r"));
    }

    #[test]
    fn long_commands_end_with_an_ellipsis_and_closing_paren() {
        let call = ToolCall::running(
            "bash",
            "cargo test --workspace --all-targets --all-features --locked",
        );
        let mut terminal = Terminal::new(TestBackend::new(34, 1)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::paint_header(frame.buffer_mut(), area, 0, &call);
            })
            .unwrap();

        let rendered = screen(&terminal, 34, 1);
        assert!(rendered.contains("…)") || rendered.contains("...)"));
        assert!(!rendered.contains("--locked"));
    }
}
