use super::metrics::composer_input_layout;
use super::state::ComposerState;
use super::wrap::ComposerLayout;
use crate::frontend::clip::render_clipped_with_cursor;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Widget},
};

pub struct ComposerWidget<'a> {
    state: &'a ComposerState,
}

impl<'a> ComposerWidget<'a> {
    pub fn new(state: &'a ComposerState) -> Self {
        Self { state }
    }

    fn block() -> Block<'static> {
        Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(96, 96, 96)))
    }
}

impl Widget for ComposerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Self::block();
        let inner = block.inner(area);
        block.render(area, buf);

        let layout = composer_input_layout(inner);
        if layout.wrap_width == 0 || inner.height == 0 {
            return;
        }

        let prompt_style = Style::default().fg(Color::Rgb(190, 190, 190));
        let text_style = Style::default().fg(Color::White);
        let text_layout = ComposerLayout::new(
            self.state.text(),
            self.state.cursor().min(self.state.text().len()),
            layout.wrap_width,
            inner.height as usize,
        );

        for (row, line) in text_layout
            .lines
            .iter()
            .skip(text_layout.scroll_top)
            .take(inner.height as usize)
            .enumerate()
        {
            let line_index = text_layout.scroll_top + row;
            let slice = &self.state.text()[line.start..line.end];
            let cursor_offset = (line_index == text_layout.cursor_line).then(|| {
                self.state
                    .cursor()
                    .min(line.end)
                    .saturating_sub(line.start)
            });
            render_clipped_with_cursor(
                buf,
                layout.text_x,
                inner.y + row as u16,
                slice,
                cursor_offset,
                layout.render_width,
                text_style,
            );
        }
        buf.set_string(inner.x, inner.y, ">", prompt_style);
    }
}

#[cfg(test)]
mod tests {
    use super::ComposerWidget;
    use crate::frontend::composer::ComposerState;
    use ratatui::{style::Modifier, Terminal, backend::TestBackend, layout::Rect};

    #[test]
    fn renders_the_prompt_and_keeps_text_inside_the_border() {
        let mut composer = ComposerState::default();
        composer.insert_paste("abcdefghijk");
        let mut terminal = Terminal::new(TestBackend::new(12, 3)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "─");
        assert_eq!(buffer.cell((0, 2)).unwrap().symbol(), "─");
        assert_eq!(buffer.cell((0, 1)).unwrap().symbol(), ">");
        assert_eq!(buffer.cell((2, 1)).unwrap().symbol(), "j");
        assert_eq!(buffer.cell((3, 1)).unwrap().symbol(), "k");
        assert!(buffer.cell((4, 1)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn keeps_inverse_cursor_on_a_full_line_until_the_next_character() {
        let mut composer = ComposerState::default();
        composer.insert_paste("123456789");
        assert_eq!(composer.desired_height(9, 20), 3);

        let mut terminal = Terminal::new(TestBackend::new(12, 3)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((10, 1)).unwrap().symbol(), "9");
        assert!(buffer.cell((11, 1)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn renders_a_wide_glyph_with_room_for_the_inverse_cursor() {
        let mut composer = ComposerState::default();
        composer.insert_paste("界");
        let mut terminal = Terminal::new(TestBackend::new(8, 3)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((2, 1)).unwrap().symbol(), "界");
        assert!(buffer.cell((4, 1)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn multiline_composer_grows_and_positions_the_cursor_on_the_active_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("one\ntwo");
        assert_eq!(composer.desired_height(8, 20), 4);

        let mut terminal = Terminal::new(TestBackend::new(12, 4)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((2, 1)).unwrap().symbol(), "o");
        assert_eq!(buffer.cell((2, 2)).unwrap().symbol(), "t");
        assert!(buffer.cell((5, 2)).unwrap().modifier.contains(Modifier::REVERSED));
    }
}
