use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectDockLayout {
    pub content_width: usize,
    pub header: Rect,
    pub label: Option<Rect>,
    pub composer: Rect,
    pub palette: Option<Rect>,
    pub footer: Option<Rect>,
}

pub fn connect_docked_layout(
    area: Rect,
    palette_height: u16,
    label_height: u16,
    footer_height: u16,
) -> ConnectDockLayout {
    if area.width < 5 || area.height < 4 {
        return ConnectDockLayout {
            content_width: 0,
            header: area,
            label: None,
            composer: area,
            palette: None,
            footer: None,
        };
    }

    let content_width = area.width.saturating_sub(3) as usize;
    let header_height = 1u16;
    let composer_height = 3u16;
    let footer_height = footer_height.min(area.height);
    let label_height = label_height.min(
        area.height
            .saturating_sub(header_height)
            .saturating_sub(composer_height)
            .saturating_sub(footer_height),
    );
    let palette_height = palette_height.min(
        area.height
            .saturating_sub(header_height)
            .saturating_sub(label_height)
            .saturating_sub(composer_height)
            .saturating_sub(footer_height),
    );

    let bottom = area.y + area.height;
    let footer = (footer_height > 0).then_some(Rect {
        x: area.x,
        y: bottom.saturating_sub(footer_height),
        width: area.width,
        height: footer_height,
    });
    let palette_top = footer
        .map(|rect| rect.y)
        .unwrap_or(bottom)
        .saturating_sub(palette_height);
    let palette = (palette_height > 0).then_some(Rect {
        x: area.x,
        y: palette_top,
        width: area.width,
        height: palette_height,
    });
    let composer = Rect {
        x: area.x,
        y: palette_top.saturating_sub(composer_height),
        width: area.width,
        height: composer_height,
    };
    let label = (label_height > 0).then_some(Rect {
        x: area.x,
        y: composer.y.saturating_sub(label_height),
        width: area.width,
        height: label_height,
    });
    let header = Rect {
        x: area.x,
        y: label
            .map(|rect| rect.y)
            .unwrap_or(composer.y)
            .saturating_sub(header_height),
        width: area.width,
        height: header_height,
    };

    ConnectDockLayout {
        content_width,
        header,
        label,
        composer,
        palette,
        footer,
    }
}

#[cfg(test)]
mod tests {
    use super::connect_docked_layout;
    use ratatui::layout::Rect;

    #[test]
    fn palette_sits_below_the_composer() {
        let layout = connect_docked_layout(Rect::new(0, 0, 48, 14), 6, 0, 0);
        let palette = layout.palette.expect("palette");
        assert_eq!(layout.composer.height, 3);
        assert_eq!(palette.y, layout.composer.y + layout.composer.height);
        assert_eq!(palette.y + palette.height, 14);
        assert_eq!(layout.header.y + 1, layout.composer.y);
    }

    #[test]
    fn api_key_step_stacks_header_label_composer_and_footer() {
        let layout = connect_docked_layout(Rect::new(0, 0, 48, 10), 0, 1, 1);
        assert_eq!(layout.header.y, 4);
        assert_eq!(layout.label.expect("label").y, 5);
        assert_eq!(layout.composer.y, 6);
        assert_eq!(layout.footer.expect("footer").y, 9);
        assert!(layout.palette.is_none());
    }
}
