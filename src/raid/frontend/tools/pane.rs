use super::status::{ToolCall, ToolStatus};
use crate::frontend::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

const MAX_VISIBLE: usize = 4;
const NAME: Color = Color::Rgb(212, 176, 120);
const RUNNING: Color = Color::Rgb(80, 196, 184);
const MUTED: Color = Color::Rgb(130, 148, 150);
const FAILED: Color = Color::Rgb(212, 176, 120);

#[derive(Debug, Default)]
pub struct ToolLog {
    calls: Vec<ToolCall>,
}

impl ToolLog {
    pub fn push(&mut self, name: impl Into<String>, detail: impl Into<String>, status: ToolStatus) {
        self.calls.push(ToolCall {
            name: name.into(),
            detail: detail.into(),
            status,
        });
    }

    pub fn start(&mut self, name: impl Into<String>, detail: impl Into<String>) -> usize {
        self.push(name, detail, ToolStatus::Running);
        self.calls.len() - 1
    }

    pub fn finish(&mut self, index: usize, status: ToolStatus) {
        if let Some(call) = self.calls.get_mut(index) {
            call.status = status;
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub fn desired_height(&self, max_height: u16) -> u16 {
        if self.calls.is_empty() || max_height == 0 {
            return 0;
        }
        (self.calls.len().min(MAX_VISIBLE) as u16).min(max_height)
    }

    pub fn widget(&self) -> ToolPaneWidget<'_> {
        let start = self.calls.len().saturating_sub(MAX_VISIBLE);
        ToolPaneWidget {
            calls: &self.calls[start..],
        }
    }
}

pub struct ToolPaneWidget<'a> {
    calls: &'a [ToolCall],
}

impl Widget for ToolPaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        for (row, call) in self
            .calls
            .iter()
            .rev()
            .take(area.height as usize)
            .enumerate()
        {
            render_call(buf, area, area.y + row as u16, call);
        }
    }
}

fn render_call(buf: &mut Buffer, area: Rect, y: u16, call: &ToolCall) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }
    let (icon_style, name_style, status_style) = styles(call.status);
    let icon = call.status.icon();
    let status = call.status.label();
    render_clipped(buf, area.x, y, icon, width, icon_style);

    let name_x = area.x.saturating_add(2);
    if name_x >= area.x + area.width {
        return;
    }
    render_clipped(
        buf,
        name_x,
        y,
        &call.name,
        width.saturating_sub(2),
        name_style,
    );

    let name_width = ratatui::text::Line::from(call.name.as_str()).width();
    let detail_x = name_x.saturating_add((name_width + 2) as u16);
    let status_width = ratatui::text::Line::from(status).width();
    let status_x = area
        .x
        .saturating_add(area.width)
        .saturating_sub(status_width as u16);
    if detail_x < status_x.saturating_sub(1) {
        let detail_width = status_x.saturating_sub(detail_x + 1) as usize;
        render_clipped(buf, detail_x, y, &call.detail, detail_width, status_style);
    }
    if status_x >= area.x {
        render_clipped(
            buf,
            status_x,
            y,
            status,
            status_width.min(width),
            status_style,
        );
    }
}

fn styles(status: ToolStatus) -> (Style, Style, Style) {
    match status {
        ToolStatus::Running => (
            Style::default().fg(RUNNING),
            Style::default().fg(NAME),
            Style::default().fg(RUNNING),
        ),
        ToolStatus::Success => (
            Style::default().fg(MUTED),
            Style::default().fg(NAME),
            Style::default().fg(MUTED),
        ),
        ToolStatus::Failed => (
            Style::default().fg(FAILED),
            Style::default().fg(NAME),
            Style::default().fg(FAILED),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::ToolLog;
    use crate::frontend::tools::ToolStatus;
    use ratatui::{Terminal, backend::TestBackend, widgets::Widget};

    #[test]
    fn desired_height_is_zero_until_a_call_exists() {
        let mut tools = ToolLog::default();
        assert_eq!(tools.desired_height(10), 0);
        tools.start("read", "src/main.rs");
        assert_eq!(tools.desired_height(10), 1);
        for index in 0..6 {
            tools.push(format!("t{index}"), "x", ToolStatus::Success);
        }
        assert_eq!(tools.desired_height(10), 4);
    }

    #[test]
    fn pane_renders_icon_name_and_status() {
        let mut tools = ToolLog::default();
        tools.push("read", "Cargo.toml", ToolStatus::Success);
        tools.push("bash", "cargo test", ToolStatus::Running);
        let mut terminal = Terminal::new(TestBackend::new(42, 2)).unwrap();
        terminal
            .draw(|frame| {
                tools.widget().render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut top = String::new();
        let mut bottom = String::new();
        for x in 0..42 {
            top.push_str(buffer.cell((x, 0)).unwrap().symbol());
            bottom.push_str(buffer.cell((x, 1)).unwrap().symbol());
        }
        assert!(top.contains("bash") && top.contains("running"));
        assert!(bottom.contains("read") && bottom.contains("done"));
    }

    #[test]
    fn finish_replaces_running_with_the_final_status() {
        let mut tools = ToolLog::default();
        let index = tools.start("bash", "boom");
        tools.finish(index, ToolStatus::Failed);
        let mut terminal = Terminal::new(TestBackend::new(32, 1)).unwrap();
        terminal
            .draw(|frame| {
                tools.widget().render(frame.area(), frame.buffer_mut());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut row = String::new();
        for x in 0..32 {
            row.push_str(buffer.cell((x, 0)).unwrap().symbol());
        }
        assert!(row.contains("bash") && row.contains("failed"));
        assert!(!row.contains("running"));
    }
}
