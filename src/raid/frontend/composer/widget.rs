use super::clip::render_clipped;
use super::state::ComposerState;
use super::wrap::ComposerLayout;
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

    pub fn cursor_position(area: Rect, state: &ComposerState) -> Option<(u16, u16)> {
        let inner = Self::block().inner(area);
        let content_width = inner.width.saturating_sub(3) as usize;
        if content_width == 0 || inner.height == 0 {
            return None;
        }

        let text_x = inner.x.saturating_add(2);
        let layout = ComposerLayout::new(
            state.text(),
            state.cursor().min(state.text().len()),
            content_width,
            inner.height as usize,
        );
        let cursor_row = layout.cursor_line.saturating_sub(layout.scroll_top);
        let cursor_x = text_x.saturating_add(
            layout
                .cursor_width
                .min(content_width.saturating_sub(1))
                .try_into()
                .unwrap_or(u16::MAX),
        );
        Some((cursor_x, inner.y + cursor_row as u16))
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

        let content_width = inner.width.saturating_sub(3) as usize;
        if content_width == 0 || inner.height == 0 {
            return;
        }

        let text_x = inner.x.saturating_add(2);
        let prompt_style = Style::default().fg(Color::Rgb(190, 190, 190));
        let text_style = Style::default().fg(Color::White);
        let layout = ComposerLayout::new(
            self.state.text(),
            self.state.cursor().min(self.state.text().len()),
            content_width,
            inner.height as usize,
        );

        for (row, line) in layout
            .lines
            .iter()
            .skip(layout.scroll_top)
            .take(inner.height as usize)
            .enumerate()
        {
            render_clipped(
                buf,
                text_x,
                inner.y + row as u16,
                &self.state.text()[line.start..line.end],
                content_width,
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
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

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
        assert_eq!(buffer.cell((11, 1)).unwrap().symbol(), " ");
        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 3), &composer),
            Some((4, 1))
        );
    }

    #[test]
    fn creates_a_caret_row_when_text_fills_the_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("123456789");
        assert_eq!(composer.desired_height(9, 20), 4);

        let mut terminal = Terminal::new(TestBackend::new(12, 4)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 4), &composer),
            Some((2, 2))
        );
    }

    #[test]
    fn renders_a_placeholder_for_a_wide_glyph_on_a_one_cell_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("界");
        let mut terminal = Terminal::new(TestBackend::new(4, 4)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        assert_eq!(
            terminal.backend().buffer().cell((2, 1)).unwrap().symbol(),
            "…"
        );
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
        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 4), &composer),
            Some((5, 2))
        );
    }
}
