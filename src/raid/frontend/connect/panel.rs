use crate::config::{AuthStore, ConnectProvider, PROVIDERS};
use crate::frontend::clip::render_clipped;
use crate::frontend::composer::{paint_input_editor, padded_input_layout, ComposerState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Widget,
};

const BORDER: Color = Color::Rgb(72, 92, 128);
const ACCENT: Color = Color::Rgb(80, 196, 184);
const TEXT: Color = Color::Rgb(228, 228, 228);
const DIM: Color = Color::Rgb(130, 148, 150);
const MUTED: Color = Color::Rgb(96, 96, 96);
const NAME: Color = Color::Rgb(212, 176, 120);
const NAME_SELECTED: Color = Color::Rgb(80, 196, 184);

pub struct ConnectPanelWidget<'a> {
    step: ConnectPanelStep,
    provider_selected: usize,
    header: &'a str,
    label: Option<&'a str>,
    footer: Option<&'a str>,
    input: Option<&'a ComposerState>,
    input_height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectPanelStep {
    Provider,
    ApiKey,
}

impl<'a> ConnectPanelWidget<'a> {
    pub fn provider(header: &'a str, selected: usize) -> Self {
        Self {
            step: ConnectPanelStep::Provider,
            provider_selected: selected,
            header,
            label: None,
            footer: None,
            input: None,
            input_height: 0,
        }
    }

    pub fn api_key(
        header: &'a str,
        label: &'a str,
        footer: &'a str,
        input: &'a ComposerState,
        input_height: u16,
    ) -> Self {
        Self {
            step: ConnectPanelStep::ApiKey,
            provider_selected: 0,
            header,
            label: Some(label),
            footer: Some(footer),
            input: Some(input),
            input_height: input_height.max(1),
        }
    }
}

impl Widget for ConnectPanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        paint_border_line(buf, area, area.y, BORDER);
        let bottom = area.y + area.height.saturating_sub(1);
        if bottom > area.y {
            paint_border_line(buf, area, bottom, BORDER);
        }

        let mut row = area.y.saturating_add(1);
        row = paint_text_row(buf, area, row, self.header, Style::default().fg(ACCENT));

        match self.step {
            ConnectPanelStep::Provider => {
                row = paint_prompt_row(buf, area, row, "");
                row = paint_provider_list(buf, area, row, self.provider_selected);
                let footer = format!(
                    "({}/{})  enter connect  esc cancel",
                    self.provider_selected + 1,
                    PROVIDERS.len().max(1)
                );
                let _ = paint_text_row(buf, area, row, &footer, Style::default().fg(MUTED));
            }
            ConnectPanelStep::ApiKey => {
                if let Some(label) = self.label {
                    row = paint_text_row(buf, area, row, label, Style::default().fg(TEXT));
                }
                if let Some(input) = self.input {
                    row = paint_api_key_input(buf, area, row, input, self.input_height);
                }
                if let Some(footer) = self.footer {
                    let _ = paint_text_row(buf, area, row, footer, Style::default().fg(MUTED));
                }
            }
        }
    }
}

fn paint_border_line(buf: &mut Buffer, area: Rect, y: u16, color: Color) {
    let width = area.width as usize;
    let line = "─".repeat(width);
    render_clipped(
        buf,
        area.x,
        y,
        &line,
        width,
        Style::default().fg(color),
    );
}

fn paint_text_row(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) -> u16 {
    render_clipped(buf, area.x.saturating_add(2), y, text, area.width as usize, style);
    y.saturating_add(1)
}

fn paint_prompt_row(buf: &mut Buffer, area: Rect, y: u16, value: &str) -> u16 {
    let x = area.x.saturating_add(2);
    render_clipped(buf, x, y, "> ", 2, Style::default().fg(TEXT));
    render_clipped(
        buf,
        x.saturating_add(2),
        y,
        value,
        area.width.saturating_sub(4) as usize,
        Style::default().fg(TEXT),
    );
    y.saturating_add(1)
}

fn paint_api_key_input(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    input: &ComposerState,
    visible_height: u16,
) -> u16 {
    let row_layout = padded_input_layout(area);
    if row_layout.wrap_width == 0 || visible_height == 0 {
        return y.saturating_add(visible_height.max(1));
    }

    paint_input_editor(
        buf,
        row_layout,
        input,
        y,
        visible_height as usize,
        Style::default().fg(TEXT),
        Style::default().fg(TEXT),
    );
    y.saturating_add(visible_height)
}

fn paint_provider_list(buf: &mut Buffer, area: Rect, mut y: u16, selected: usize) -> u16 {
    let visible = PROVIDERS.len().min(8).max(1);
    let scroll_top = scroll_top(selected, PROVIDERS.len(), visible);
    let name_col = PROVIDERS
        .iter()
        .map(|provider| Line::from(provider.label).width())
        .max()
        .unwrap_or(0);

    for (row, provider) in PROVIDERS
        .iter()
        .skip(scroll_top)
        .take(visible)
        .enumerate()
    {
        if y >= area.y + area.height.saturating_sub(2) {
            break;
        }
        paint_provider_row(
            buf,
            area,
            y,
            provider,
            scroll_top + row == selected,
            name_col,
        );
        y = y.saturating_add(1);
    }
    y
}

fn paint_provider_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    provider: &ConnectProvider,
    selected: bool,
    name_col: usize,
) {
    let marker = if selected { "→" } else { " " };
    let name_style = if selected {
        Style::default().fg(NAME_SELECTED)
    } else {
        Style::default().fg(NAME)
    };
    let status = if AuthStore::load().has_provider(provider.id) {
        "configured"
    } else {
        "unconfigured"
    };
    let mut x = area.x.saturating_add(2);
    render_clipped(buf, x, y, marker, area.width as usize, name_style);
    x = x.saturating_add(2);
    render_clipped(buf, x, y, provider.label, area.width as usize, name_style);
    x = x.saturating_add(name_col as u16 + 1);
    render_clipped(buf, x, y, "·", 2, Style::default().fg(DIM));
    x = x.saturating_add(2);
    render_clipped(
        buf,
        x,
        y,
        status,
        area.width.saturating_sub(x - area.x) as usize,
        Style::default().fg(DIM),
    );
}

fn scroll_top(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible {
        return 0;
    }
    if selected < visible {
        0
    } else if selected + visible > total {
        total.saturating_sub(visible)
    } else {
        selected.saturating_sub(visible / 2)
    }
}

pub fn panel_height(
    step: ConnectPanelStep,
    area_width: u16,
    max_height: u16,
    input: &ComposerState,
) -> u16 {
    match step {
        ConnectPanelStep::Provider => provider_panel_height(),
        ConnectPanelStep::ApiKey => api_key_panel_height(area_width, max_height, input),
    }
}

pub fn connect_input_wrap_width(area_width: u16) -> usize {
    padded_input_layout(Rect::new(0, 0, area_width.max(1), 1))
        .wrap_width
        .max(1)
}

fn provider_panel_height() -> u16 {
    let list_rows = PROVIDERS.len().min(8).max(1) as u16;
    2 + 1 + 1 + list_rows + 1
}

fn api_key_panel_height(area_width: u16, max_height: u16, input: &ComposerState) -> u16 {
    let wrap_width = connect_input_wrap_width(area_width);
    let max_lines = max_height
        .saturating_sub(5)
        .max(1)
        .min(4) as usize;
    let line_count = crate::frontend::composer::visual_lines_for_cursor(
        input.text(),
        input.cursor(),
        wrap_width,
    )
    .len()
    .clamp(1, max_lines);
    (5 + line_count) as u16
}

#[cfg(test)]
mod tests {
    use super::{
        api_key_panel_height, connect_input_wrap_width, ConnectPanelStep, ConnectPanelWidget,
        panel_height,
    };
    use crate::frontend::composer::ComposerState;
    use ratatui::{backend::TestBackend, layout::Rect, style::Modifier, Terminal};

    #[test]
    fn provider_panel_puts_list_below_prompt() {
        let widget = ConnectPanelWidget::provider("Select provider to configure:", 0);
        let input = ComposerState::default();
        let mut terminal = Terminal::new(TestBackend::new(56, 12)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    widget,
                    Rect::new(0, 4, 56, panel_height(ConnectPanelStep::Provider, 56, 12, &input)),
                );
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
        assert!(row(5).contains("Select provider"));
        assert!(row(6).contains('>'));
        assert!(row(7).contains("OpenCode"));
        assert!(row(5).find('>').unwrap_or(0) < row(7).find("OpenCode").unwrap_or(56));
    }

    #[test]
    fn api_key_panel_renders_multiline_input() {
        let mut input = ComposerState::default();
        input.insert_paste("line-one\nline-two");
        let height = api_key_panel_height(40, 12, &input);
        let widget = ConnectPanelWidget::api_key(
            "Connect to OpenCode Go",
            "Enter OpenCode API key",
            "(shift+enter newline, enter submit, esc cancel)",
            &input,
            height.saturating_sub(5),
        );
        let mut terminal = Terminal::new(TestBackend::new(56, height)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            let mut text = String::new();
            for x in 0..56 {
                text.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            text
        };
        assert!(row(3).contains('>'));
        assert!(row(3).contains("line-one"));
        assert!(row(4).contains("line-two"));
    }

    #[test]
    fn api_key_input_keeps_inverse_cursor_on_a_full_line() {
        let mut input = ComposerState::default();
        input.insert_paste("123456789");
        let width = 56u16;
        let wrap_width = connect_input_wrap_width(width);
        assert_eq!(wrap_width, 49);
        let height = api_key_panel_height(width, 12, &input);
        let widget = ConnectPanelWidget::api_key(
            "Connect to OpenCode Go",
            "Enter OpenCode API key",
            "(shift+enter newline, enter submit, esc cancel)",
            &input,
            height.saturating_sub(5),
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let input_y = 3;
        assert_eq!(buffer.cell((12, input_y)).unwrap().symbol(), "9");
        assert!(
            buffer
                .cell((13, input_y))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
    }
}
