use super::registry::SlashCommand;
use crate::frontend::composer::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Widget,
};

const NAME: Color = Color::Rgb(212, 176, 120);
const NAME_SELECTED: Color = Color::Rgb(80, 196, 184);
const DESCRIPTION: Color = Color::Rgb(130, 148, 150);
const FOOTER: Color = Color::Rgb(96, 96, 96);

pub struct SlashPaletteWidget {
    matches: Vec<&'static SlashCommand>,
    selected: usize,
}

impl SlashPaletteWidget {
    pub fn new(matches: Vec<&'static SlashCommand>, selected: usize) -> Self {
        let selected = if matches.is_empty() {
            0
        } else {
            selected.min(matches.len() - 1)
        };
        Self { matches, selected }
    }
}

impl Widget for SlashPaletteWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let has_footer = area.height >= 2;
        let item_rows = if has_footer {
            (area.height - 1) as usize
        } else {
            area.height as usize
        };
        let item_rows = item_rows.max(1);

        if self.matches.is_empty() {
            render_clipped(
                buf,
                area.x,
                area.y,
                "no matching commands",
                area.width as usize,
                Style::default().fg(DESCRIPTION),
            );
        } else {
            let scroll_top = scroll_top(self.selected, self.matches.len(), item_rows);
            let name_col = self
                .matches
                .iter()
                .map(|command| Line::from(command.name).width())
                .max()
                .unwrap_or(0);

            for (row, command) in self
                .matches
                .iter()
                .skip(scroll_top)
                .take(item_rows)
                .enumerate()
            {
                let y = area.y + row as u16;
                let selected = scroll_top + row == self.selected;
                render_command_row(buf, area, y, command, selected, name_col);
            }
        }

        if has_footer {
            let footer = if self.matches.is_empty() {
                "(0/0)".to_owned()
            } else {
                format!("({}/{})", self.selected + 1, self.matches.len())
            };
            render_clipped(
                buf,
                area.x,
                area.y + area.height - 1,
                &footer,
                area.width as usize,
                Style::default().fg(FOOTER),
            );
        }
    }
}

fn selection_prefix() -> (String, usize) {
    let arrow = "→";
    let arrow_width = Line::from(arrow).width().max(1);
    (format!("{arrow} "), arrow_width + 1)
}

fn scroll_top(selected: usize, len: usize, rows: usize) -> usize {
    if len <= rows {
        0
    } else {
        selected.saturating_sub(rows.saturating_sub(1))
    }
}

fn render_command_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    command: &SlashCommand,
    selected: bool,
    name_col: usize,
) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }

    let (arrow, prefix_width) = selection_prefix();
    let prefix = if selected {
        arrow
    } else {
        " ".repeat(prefix_width)
    };
    let name_style = if selected {
        Style::default().fg(NAME_SELECTED)
    } else {
        Style::default().fg(NAME)
    };

    render_clipped(buf, area.x, y, &prefix, width, name_style);

    let name_x = area.x.saturating_add(prefix_width as u16);
    if name_x >= area.x + area.width {
        return;
    }
    let name_space = width.saturating_sub(prefix_width);
    render_clipped(buf, name_x, y, command.name, name_space, name_style);

    let desc_x = name_x.saturating_add((name_col + 2).min(u16::MAX as usize) as u16);
    if desc_x >= area.x + area.width {
        return;
    }
    let desc_space = (area.x + area.width).saturating_sub(desc_x) as usize;
    render_clipped(
        buf,
        desc_x,
        y,
        command.description,
        desc_space,
        Style::default().fg(DESCRIPTION),
    );
}

#[cfg(test)]
mod tests {
    use super::SlashPaletteWidget;
    use crate::frontend::composer::slash_commands::{COMMANDS, matching_commands};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn palette_renders_selection_arrow_aligned_columns_and_footer() {
        let matches = matching_commands("");
        let mut terminal = Terminal::new(TestBackend::new(72, 6)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(SlashPaletteWidget::new(matches.clone(), 0), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "→");
        assert!(buffer_row(buffer, 0).contains("settings"));
        assert!(buffer_row(buffer, 0).contains("Open settings menu"));
        assert!(buffer_row(buffer, 1).contains("model"));
        assert!(!buffer_row(buffer, 1).contains("→"));
        assert_eq!(
            buffer_row(buffer, 5).trim(),
            format!("(1/{})", COMMANDS.len())
        );
    }

    #[test]
    fn palette_scrolls_so_the_selected_row_stays_visible() {
        let matches = matching_commands("");
        let mut terminal = Terminal::new(TestBackend::new(48, 6)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(SlashPaletteWidget::new(matches.clone(), 6), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let visible = (0..5)
            .map(|row| buffer_row(buffer, row))
            .collect::<String>();
        assert!(visible.contains("compact"));
        assert!(!visible.contains("settings"));
        assert_eq!(
            buffer_row(buffer, 5).trim(),
            format!("(7/{})", COMMANDS.len())
        );
    }

    #[test]
    fn empty_matches_render_a_placeholder_and_zero_footer() {
        let mut terminal = Terminal::new(TestBackend::new(32, 3)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(SlashPaletteWidget::new(Vec::new(), 0), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer_row(buffer, 0).contains("no matching commands"));
        assert_eq!(buffer_row(buffer, 2).trim(), "(0/0)");
    }

    fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut row = String::new();
        for x in 0..buffer.area.width {
            row.push_str(buffer.cell((x, y)).unwrap().symbol());
        }
        row
    }
}
