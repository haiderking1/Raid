use super::card::{paint_header, paint_result};
use super::status::{ToolCall, ToolStatus};
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

const MAX_CARDS: usize = 4;

#[derive(Debug, Default)]
#[cfg_attr(not(test), expect(dead_code))]
pub struct ToolLog {
    calls: Vec<ToolCall>,
}

#[cfg_attr(not(test), expect(dead_code))]
impl ToolLog {
    pub fn push(&mut self, name: impl Into<String>, detail: impl Into<String>, status: ToolStatus) {
        self.calls.push(ToolCall::new(name, detail, status));
    }

    pub fn start(&mut self, name: impl Into<String>, detail: impl Into<String>) -> usize {
        self.calls.push(ToolCall::running(name, detail));
        self.calls.len() - 1
    }

    pub fn finish(&mut self, index: usize, status: ToolStatus) {
        if let Some(call) = self.calls.get_mut(index) {
            let summary = ToolCall::new(&call.name, &call.detail, status).summary;
            call.finish(status, summary);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    pub fn desired_height(&self, max_height: u16) -> u16 {
        if self.calls.is_empty() || max_height == 0 {
            return 0;
        }
        let rows = self.calls.len().min(MAX_CARDS).saturating_mul(2);
        (rows as u16).min(max_height)
    }

    pub fn widget(&self) -> ToolPaneWidget<'_> {
        let start = self.calls.len().saturating_sub(MAX_CARDS);
        ToolPaneWidget {
            calls: &self.calls[start..],
        }
    }
}

#[cfg_attr(not(test), expect(dead_code))]
pub struct ToolPaneWidget<'a> {
    calls: &'a [ToolCall],
}

impl Widget for ToolPaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);
        for call in self.calls.iter().rev() {
            if y >= bottom {
                break;
            }
            paint_header(buf, area, y, call);
            y = y.saturating_add(1);
            if y >= bottom {
                break;
            }
            paint_result(buf, area, y, call);
            y = y.saturating_add(1);
        }
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
        assert!(tools.is_empty());
        assert_eq!(tools.desired_height(10), 0);
        tools.start("read", "src/main.rs");
        assert_eq!(tools.desired_height(10), 2);
        for index in 0..6 {
            tools.push(format!("t{index}"), "x", ToolStatus::Success);
        }
        assert_eq!(tools.desired_height(10), 8);
    }

    #[test]
    fn pane_renders_newest_card_first() {
        let mut tools = ToolLog::default();
        tools.push("read", "Cargo.toml", ToolStatus::Success);
        tools.push("bash", "cargo test", ToolStatus::Running);
        let mut terminal = Terminal::new(TestBackend::new(42, 4)).unwrap();
        terminal
            .draw(|frame| {
                tools.widget().render(frame.area(), frame.buffer_mut());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut rows = [String::new(), String::new(), String::new(), String::new()];
        for x in 0..42 {
            for (index, row) in rows.iter_mut().enumerate() {
                row.push_str(buffer.cell((x, index as u16)).unwrap().symbol());
            }
        }
        assert!(rows[0].contains("Bash(cargo test)"));
        assert!(rows[1].contains("└ Running"));
        assert!(rows[2].contains("Read(Cargo.toml)"));
        assert!(rows[3].contains("└"));
        assert!(rows[3].contains("Read"));
        assert!(!rows[3].contains("ctrl+"));
    }

    #[test]
    fn finish_replaces_running_with_the_final_status() {
        let mut tools = ToolLog::default();
        let index = tools.start("bash", "boom");
        tools.finish(index, ToolStatus::Failed);
        let mut terminal = Terminal::new(TestBackend::new(32, 2)).unwrap();
        terminal
            .draw(|frame| {
                tools.widget().render(frame.area(), frame.buffer_mut());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut header = String::new();
        let mut result = String::new();
        for x in 0..32 {
            header.push_str(buffer.cell((x, 0)).unwrap().symbol());
            result.push_str(buffer.cell((x, 1)).unwrap().symbol());
        }
        assert!(header.contains("Bash(boom)"));
        assert!(result.contains("Failed"));
        assert!(!result.contains("Running"));
    }
}
