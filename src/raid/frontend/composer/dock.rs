use super::state::ComposerState;
use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockedLayout {
    pub content_width: usize,
    pub composer: Rect,
    pub palette: Option<Rect>,
}

pub fn docked_layout(area: Rect, state: &ComposerState) -> DockedLayout {
    if area.width < 5 || area.height < 3 {
        return DockedLayout {
            content_width: 0,
            composer: area,
            palette: None,
        };
    }

    let content_width = area.width.saturating_sub(3) as usize;
    let palette_desired = state.palette_height(area.height.saturating_sub(3));
    let composer_height =
        state.desired_height(content_width, area.height.saturating_sub(palette_desired));
    let palette_height = state.palette_height(area.height.saturating_sub(composer_height));
    let composer = Rect {
        x: area.x,
        y: area.y + area.height - composer_height - palette_height,
        width: area.width,
        height: composer_height,
    };
    let palette = (palette_height > 0).then_some(Rect {
        x: area.x,
        y: composer.y + composer_height,
        width: area.width,
        height: palette_height,
    });

    DockedLayout {
        content_width,
        composer,
        palette,
    }
}

#[cfg(test)]
mod tests {
    use super::docked_layout;
    use crate::frontend::composer::slash_commands::COMMANDS;
    use crate::frontend::composer::{ComposerState, ComposerWidget};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    #[test]
    fn docked_layout_places_the_palette_under_the_composer() {
        let mut composer = ComposerState::default();
        composer.insert_paste("/");
        let layout = docked_layout(Rect::new(0, 0, 48, 14), &composer);

        assert_eq!(layout.composer.height, 3);
        let palette = layout.palette.expect("palette should be visible");
        assert_eq!(palette.y, layout.composer.y + layout.composer.height);
        assert_eq!(palette.height, COMMANDS.len() as u16 + 1);
        assert_eq!(
            layout.composer.y + layout.composer.height + palette.height,
            14
        );
    }

    #[test]
    fn slash_palette_renders_under_the_composer() {
        let mut composer = ComposerState::default();
        composer.insert_paste("/");
        let area = Rect::new(0, 0, 56, 12);
        let layout = docked_layout(area, &composer);
        let palette = layout.palette.expect("palette should be visible");
        let composer_line = layout.composer.y + 1;
        let palette_line = palette.y;
        let footer_line = palette.y + palette.height - 1;
        let mut terminal = Terminal::new(TestBackend::new(56, 12)).unwrap();

        terminal
            .draw(|frame| {
                let layout = docked_layout(frame.area(), &composer);
                frame.render_widget(ComposerWidget::new(&composer), layout.composer);
                if let (Some(area), Some(widget)) = (layout.palette, composer.palette_widget()) {
                    frame.render_widget(widget, area);
                }
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            let mut text = String::new();
            for x in 0..56 {
                text.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            text
        };

        assert!(row(composer_line).contains('>'));
        assert!(row(composer_line).contains('/'));
        assert!(row(palette_line).contains("connect"));
        assert!(row(footer_line).contains(&format!("(1/{})", COMMANDS.len())));
    }
}
