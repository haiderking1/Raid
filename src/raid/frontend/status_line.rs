use std::path::{Path, PathBuf};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;

const ACCENT: Color = Color::Rgb(80, 196, 184);
const TEXT: Color = Color::Rgb(130, 148, 150);
const MUTED: Color = Color::Rgb(96, 96, 96);
const PATH: Color = Color::Rgb(139, 180, 250);
const WARNING: Color = Color::Rgb(212, 176, 120);

pub struct StatusLineWidget<'a> {
    model: &'a str,
    context_tokens: u64,
    context_limit: u64,
    thinking: &'a str,
    workspace: &'a Path,
}

impl<'a> StatusLineWidget<'a> {
    pub fn new(
        model: &'a str,
        context_tokens: u64,
        context_limit: u64,
        thinking: &'a str,
        workspace: &'a Path,
    ) -> Self {
        Self {
            model,
            context_tokens,
            context_limit,
            thinking,
            workspace,
        }
    }
}

impl Widget for StatusLineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let max_path_width = (area.width as usize / 3).max(1);
        let path = workspace_label(self.workspace, max_path_width);
        let path_width = Line::from(path.as_str()).width().min(area.width as usize) as u16;
        let gap = u16::from(path_width > 0 && area.width > path_width);
        let left_width = area.width.saturating_sub(path_width).saturating_sub(gap);

        let percent = context_percent(self.context_tokens, self.context_limit);
        let context_style = Style::default().fg(if percent >= 80 { WARNING } else { TEXT });
        let full = left_width >= 46;
        let details = if full {
            format!("  ·  context {percent}%  ·  thinking {}", self.thinking)
        } else {
            format!("  ·  {percent}%  ·  {}", self.thinking)
        };
        let detail_width = Line::from(details.as_str()).width();
        let model = clip_from_right(
            self.model,
            (left_width as usize).saturating_sub(detail_width),
        );
        let separator = Span::styled("  ·  ", Style::default().fg(MUTED));
        let line = if full {
            Line::from(vec![
                Span::styled(model, Style::default().fg(ACCENT)),
                separator.clone(),
                Span::styled("context ", Style::default().fg(MUTED)),
                Span::styled(format!("{percent}%"), context_style),
                separator,
                Span::styled("thinking ", Style::default().fg(MUTED)),
                Span::styled(self.thinking, Style::default().fg(TEXT)),
            ])
        } else {
            Line::from(vec![
                Span::styled(model, Style::default().fg(ACCENT)),
                separator.clone(),
                Span::styled(format!("{percent}%"), context_style),
                separator,
                Span::styled(self.thinking, Style::default().fg(TEXT)),
            ])
        };

        if left_width > 0 {
            buf.set_line(area.x, area.y, &line, left_width);
        }
        if path_width > 0 {
            let path_x = area.right().saturating_sub(path_width);
            buf.set_stringn(
                path_x,
                area.y,
                &path,
                path_width as usize,
                Style::default().fg(PATH),
            );
        }
    }
}

fn context_percent(tokens: u64, limit: u64) -> u64 {
    if limit == 0 {
        return 0;
    }
    tokens.saturating_mul(100).div_ceil(limit).min(999)
}

fn workspace_label(path: &Path, max_width: usize) -> String {
    let display = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf))
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", relative.display())
            }
        })
        .unwrap_or_else(|| path.display().to_string());

    if Line::from(display.as_str()).width() <= max_width {
        return display;
    }
    let basename = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| path.as_os_str().to_string_lossy());
    clip_from_left(&format!("…/{basename}"), max_width)
}

fn clip_from_left(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if Line::from(text).width() <= max_width {
        return text.to_string();
    }

    let marker = "…";
    let marker_width = Line::from(marker).width();
    if marker_width >= max_width {
        return marker.to_string();
    }
    let available = max_width - marker_width;
    let mut tail = Vec::new();
    let mut used = 0;
    for grapheme in text.graphemes(true).rev() {
        let width = Line::from(grapheme).width();
        if used + width > available {
            break;
        }
        tail.push(grapheme);
        used += width;
    }
    tail.reverse();
    format!("{marker}{}", tail.concat())
}

fn clip_from_right(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if Line::from(text).width() <= max_width {
        return text.to_string();
    }

    let marker = "…";
    let marker_width = Line::from(marker).width();
    if marker_width >= max_width {
        return marker.to_string();
    }
    let available = max_width - marker_width;
    let mut head = Vec::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let width = Line::from(grapheme).width();
        if used + width > available {
            break;
        }
        head.push(grapheme);
        used += width;
    }
    format!("{}{marker}", head.concat())
}

#[cfg(test)]
mod tests {
    use super::StatusLineWidget;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::Path;

    #[test]
    fn renders_all_four_status_values() {
        let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(
                    StatusLineWidget::new(
                        "gpt-5.6-sol",
                        32_000,
                        128_000,
                        "default",
                        Path::new("/work/Raid"),
                    ),
                    frame.area(),
                );
            })
            .unwrap();

        let row = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(row.contains("gpt-5.6-sol"));
        assert!(row.contains("context 25%"));
        assert!(row.contains("thinking default"));
        assert!(row.contains("/work/Raid"));
    }

    #[test]
    fn keeps_every_indicator_when_the_model_name_is_long() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(
                    StatusLineWidget::new(
                        "provider-model-with-a-very-long-name",
                        90_000,
                        100_000,
                        "default",
                        Path::new("/Raid"),
                    ),
                    frame.area(),
                );
            })
            .unwrap();

        let row = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(row.contains("provider"));
        assert!(row.contains("context 90%"));
        assert!(row.contains("thinking default"));
        assert!(row.contains("/Raid"));
    }
}
