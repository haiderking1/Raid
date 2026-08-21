use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy)]
pub struct VisualLine {
    pub start: usize,
    pub end: usize,
}

pub fn previous_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

pub fn next_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}

pub fn cursor_visual_column(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    let line = lines[cursor_line(&lines, cursor)];
    Line::from(&text[line.start..cursor.min(line.end)]).width()
}

pub fn visual_line_start(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    lines[cursor_line(&lines, cursor)].start
}

pub fn visual_line_end(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    lines[cursor_line(&lines, cursor)].end
}

pub fn move_vertical(text: &str, cursor: usize, width: usize, up: bool, column: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    let current_line = cursor_line(&lines, cursor);
    let target_line = if up {
        current_line.checked_sub(1)
    } else {
        current_line
            .checked_add(1)
            .filter(|&line| line < lines.len())
    };

    target_line.map_or(cursor, |line| {
        position_in_line(text, lines[line].start, lines[line].end, column)
    })
}

fn position_in_line(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut position = start;
    let mut width = 0;
    for (offset, grapheme) in text[start..end].grapheme_indices(true) {
        let grapheme_width = Line::from(grapheme).width();
        if width + grapheme_width > column {
            break;
        }
        width += grapheme_width;
        position = start + offset + grapheme.len();
    }
    position
}

pub fn visual_lines(text: &str, width: usize) -> Vec<VisualLine> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut logical_start = 0;

    for logical_line in text.split('\n') {
        let logical_end = logical_start + logical_line.len();
        if logical_line.is_empty() {
            lines.push(VisualLine {
                start: logical_start,
                end: logical_end,
            });
        } else {
            let mut segment_start = logical_start;
            let mut segment_width = 0;
            for (offset, grapheme) in logical_line.grapheme_indices(true) {
                let grapheme_width = Line::from(grapheme).width();
                if segment_width > 0 && segment_width + grapheme_width > width {
                    lines.push(VisualLine {
                        start: segment_start,
                        end: logical_start + offset,
                    });
                    segment_start = logical_start + offset;
                    segment_width = 0;
                }
                segment_width += grapheme_width;
            }
            lines.push(VisualLine {
                start: segment_start,
                end: logical_end,
            });
        }
        logical_start = logical_end + '\n'.len_utf8();
    }

    lines
}

pub fn visual_lines_for_cursor(text: &str, _cursor: usize, width: usize) -> Vec<VisualLine> {
    visual_lines(text, width.max(1))
}

pub struct ComposerLayout {
    pub lines: Vec<VisualLine>,
    pub cursor_line: usize,
    pub scroll_top: usize,
}

impl ComposerLayout {
    pub fn new(text: &str, cursor: usize, content_width: usize, visible_height: usize) -> Self {
        let lines = visual_lines_for_cursor(text, cursor, content_width);
        let cursor_line = cursor_line(&lines, cursor);
        let visible_height = visible_height.max(1).min(lines.len());
        let scroll_top = cursor_line.saturating_sub(visible_height - 1);

        Self {
            lines,
            cursor_line,
            scroll_top,
        }
    }
}

fn cursor_line(lines: &[VisualLine], cursor: usize) -> usize {
    for (index, line) in lines.iter().enumerate() {
        if cursor < line.end {
            return index;
        }
        if cursor == line.end {
            let is_soft_wrap = lines
                .get(index + 1)
                .is_some_and(|next| next.start == cursor);
            if !is_soft_wrap {
                return index;
            }
        }
    }
    lines.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{visual_lines, visual_lines_for_cursor};

    #[test]
    fn wrapping_keeps_combining_graphemes_together() {
        let text = "e\u{301}x";
        let lines = visual_lines(text, 1);

        assert_eq!(lines.len(), 2);
        assert_eq!(&text[lines[0].start..lines[0].end], "e\u{301}");
        assert_eq!(&text[lines[1].start..lines[1].end], "x");
    }

    #[test]
    fn full_line_keeps_cursor_on_same_row_until_next_character() {
        let text = "123456789";
        assert_eq!(visual_lines_for_cursor(text, text.len(), 9).len(), 1);
    }

    #[test]
    fn next_character_on_a_full_line_soft_wraps() {
        let text = "1234567890";
        let lines = visual_lines(text, 9);
        assert_eq!(lines.len(), 2);
        assert_eq!(&text[lines[0].start..lines[0].end], "123456789");
        assert_eq!(&text[lines[1].start..lines[1].end], "0");
    }
}
