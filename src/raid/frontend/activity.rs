use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;

const SHIMMER_SECONDS: f32 = 2.0;
const SHIMMER_PADDING: usize = 10;
const SHIMMER_HALF_WIDTH: f32 = 5.0;
const BASE: (u8, u8, u8) = (130, 148, 150);
const HIGHLIGHT: (u8, u8, u8) = (235, 235, 235);
const DIM: Color = Color::Rgb(96, 96, 96);

pub struct ActivityIndicator {
    active: bool,
    header: String,
    started_at: Instant,
}

impl Default for ActivityIndicator {
    fn default() -> Self {
        Self {
            active: false,
            header: String::from("Working"),
            started_at: Instant::now(),
        }
    }
}

impl ActivityIndicator {
    pub fn sync(&mut self, header: Option<&str>) {
        let active = header.is_some();
        if active && !self.active {
            self.started_at = Instant::now();
        }
        if let Some(header) = header.filter(|header| !header.is_empty()) {
            header.clone_into(&mut self.header);
        }
        self.active = active;
    }

    pub fn widget(&self) -> Option<ActivityWidget<'_>> {
        self.active.then(|| ActivityWidget {
            header: &self.header,
            elapsed: self.started_at.elapsed(),
        })
    }
}

pub struct ActivityWidget<'a> {
    header: &'a str,
    elapsed: Duration,
}

impl Widget for ActivityWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut spans = shimmer_spans("•", self.elapsed);
        spans.push(Span::raw(" "));
        spans.extend(shimmer_spans(self.header, self.elapsed));
        spans.push(Span::styled(
            format!(
                " ({} • esc to interrupt)",
                format_elapsed(self.elapsed.as_secs())
            ),
            Style::default().fg(DIM),
        ));

        paint_clipped_line(buf, area, spans);
    }
}

fn format_elapsed(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3_600 {
        return format!("{}m {:02}s", seconds / 60, seconds % 60);
    }
    format!(
        "{}h {:02}m {:02}s",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

fn shimmer_spans(text: &str, elapsed: Duration) -> Vec<Span<'static>> {
    let characters = text.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return Vec::new();
    }

    let period = characters.len() + SHIMMER_PADDING * 2;
    let position = (elapsed.as_secs_f32() % SHIMMER_SECONDS) / SHIMMER_SECONDS * period as f32;
    characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let distance = (index as f32 + SHIMMER_PADDING as f32 - position).abs();
            let intensity = if distance <= SHIMMER_HALF_WIDTH {
                let phase = std::f32::consts::PI * distance / SHIMMER_HALF_WIDTH;
                0.5 * (1.0 + phase.cos())
            } else {
                0.0
            };
            Span::styled(
                character.to_string(),
                Style::default()
                    .fg(blend(BASE, HIGHLIGHT, intensity * 0.9))
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect()
}

fn blend(base: (u8, u8, u8), highlight: (u8, u8, u8), amount: f32) -> Color {
    let mix = |base: u8, highlight: u8| {
        (base as f32 + (highlight as f32 - base as f32) * amount.clamp(0.0, 1.0)).round() as u8
    };
    Color::Rgb(
        mix(base.0, highlight.0),
        mix(base.1, highlight.1),
        mix(base.2, highlight.2),
    )
}

fn paint_clipped_line(buf: &mut Buffer, area: Rect, spans: Vec<Span<'static>>) {
    let full_width = Line::from(spans.clone()).width();
    let clipped = full_width > area.width as usize;
    let content_width = if clipped {
        area.width.saturating_sub(1)
    } else {
        area.width
    } as usize;
    let mut x = area.x;
    let end = area.x.saturating_add(content_width as u16);

    'spans: for span in spans {
        for grapheme in span.content.graphemes(true) {
            let width = Line::from(grapheme).width() as u16;
            if x.saturating_add(width) > end {
                break 'spans;
            }
            buf.set_string(x, area.y, grapheme, span.style);
            x = x.saturating_add(width);
        }
    }

    if clipped {
        buf.set_string(
            area.right().saturating_sub(1),
            area.y,
            "…",
            Style::default().fg(DIM),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityIndicator, ActivityWidget, format_elapsed};
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Duration;

    fn rendered(widget: ActivityWidget<'_>, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn renders_working_time_and_interrupt_hint() {
        let row = rendered(
            ActivityWidget {
                header: "Working",
                elapsed: Duration::from_secs(5),
            },
            48,
        );

        assert!(row.starts_with("• Working (5s • esc to interrupt)"));
    }

    #[test]
    fn updates_the_header_without_restarting_the_run() {
        let mut indicator = ActivityIndicator::default();
        indicator.sync(Some("Working"));
        let started_at = indicator.started_at;

        indicator.sync(Some("Inspecting the project"));

        assert_eq!(indicator.header, "Inspecting the project");
        assert_eq!(indicator.started_at, started_at);
    }

    #[test]
    fn hides_after_the_run_finishes() {
        let mut indicator = ActivityIndicator::default();
        indicator.sync(Some("Working"));
        assert!(indicator.widget().is_some());

        indicator.sync(None);

        assert!(indicator.widget().is_none());
    }

    #[test]
    fn clips_narrow_rows_with_an_ellipsis() {
        let row = rendered(
            ActivityWidget {
                header: "Working",
                elapsed: Duration::ZERO,
            },
            20,
        );
        assert!(row.ends_with('…'));
    }

    #[test]
    fn elapsed_time_uses_compact_units() {
        assert_eq!(format_elapsed(59), "59s");
        assert_eq!(format_elapsed(61), "1m 01s");
        assert_eq!(format_elapsed(3_661), "1h 01m 01s");
    }
}
