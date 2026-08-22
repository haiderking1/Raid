use crate::frontend::composer::{ComposerState, docked_layout};
use ratatui::layout::Rect;

pub const THINKING_RESERVE: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellLayout {
    pub content_width: usize,
    pub chat: Rect,
    pub tools: Option<Rect>,
    pub thinking: Option<Rect>,
    pub composer: Rect,
    pub palette: Option<Rect>,
}

pub fn shell_layout(
    area: Rect,
    composer: &ComposerState,
    tools_height: u16,
    thinking_visible: bool,
) -> ShellLayout {
    let dock = docked_layout(area, composer);
    let above = dock.composer.y.saturating_sub(area.y);
    let thinking_height = if thinking_visible {
        THINKING_RESERVE.min(above)
    } else {
        0
    };
    let tools_height = tools_height.min(above.saturating_sub(thinking_height));
    let thinking = (thinking_height > 0).then_some(Rect {
        x: area.x,
        y: dock.composer.y.saturating_sub(thinking_height),
        width: area.width,
        height: thinking_height,
    });
    let band_top = thinking.map(|rect| rect.y).unwrap_or(dock.composer.y);
    let tools = (tools_height > 0).then_some(Rect {
        x: area.x,
        y: band_top.saturating_sub(tools_height),
        width: area.width,
        height: tools_height,
    });
    let chat_bottom = tools.map(|rect| rect.y).unwrap_or(band_top);
    let chat = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: chat_bottom.saturating_sub(area.y),
    };

    ShellLayout {
        content_width: dock.content_width,
        chat,
        tools,
        thinking,
        composer: dock.composer,
        palette: dock.palette,
    }
}

#[cfg(test)]
mod tests {
    use super::{THINKING_RESERVE, shell_layout};
    use crate::frontend::composer::ComposerState;
    use ratatui::layout::Rect;

    #[test]
    fn thinking_slot_sits_on_the_composer() {
        let composer = ComposerState::default();
        let layout = shell_layout(Rect::new(0, 0, 40, 20), &composer, 0, true);
        let thinking = layout.thinking.expect("thinking slot");
        assert_eq!(thinking.height, THINKING_RESERVE);
        assert_eq!(thinking.y + thinking.height, layout.composer.y);
        assert_eq!(layout.chat.height, thinking.y);
        assert!(layout.tools.is_none());
    }

    #[test]
    fn idle_layout_does_not_reserve_a_thinking_slot() {
        let composer = ComposerState::default();
        let layout = shell_layout(Rect::new(0, 0, 40, 20), &composer, 0, false);

        assert!(layout.thinking.is_none());
        assert_eq!(layout.chat.y + layout.chat.height, layout.composer.y);
    }

    #[test]
    fn chat_sits_above_tools_and_thinking() {
        let composer = ComposerState::default();
        let layout = shell_layout(Rect::new(0, 0, 40, 20), &composer, 2, true);
        assert_eq!(layout.composer.height, 3);
        let thinking = layout.thinking.expect("thinking");
        let tools = layout.tools.expect("tools");
        assert_eq!(thinking.y + thinking.height, layout.composer.y);
        assert_eq!(tools.y + tools.height, thinking.y);
        assert_eq!(layout.chat.y, 0);
        assert_eq!(layout.chat.height, tools.y);
    }
}
