use crate::backend::opencode::ResolvedModel;
use crate::config::MAX_VISIBLE_MODELS;
use crate::frontend::clip::render_clipped;
use crate::frontend::composer::{paint_input_editor, padded_input_layout, ComposerState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

const BORDER: Color = Color::Rgb(72, 92, 128);
const TEXT: Color = Color::Rgb(228, 228, 228);
const DIM: Color = Color::Rgb(130, 148, 150);
const MUTED: Color = Color::Rgb(96, 96, 96);
const NAME: Color = Color::Rgb(212, 176, 120);
const NAME_SELECTED: Color = Color::Rgb(80, 196, 184);

const FIXED_ROWS: u16 = 5;

pub struct ModelPaletteWidget<'a> {
    search: &'a ComposerState,
    models: &'a [ResolvedModel],
    filtered: &'a [usize],
    selected: usize,
    status: &'a str,
}

impl<'a> ModelPaletteWidget<'a> {
    pub fn new(
        search: &'a ComposerState,
        models: &'a [ResolvedModel],
        filtered: &'a [usize],
        selected: usize,
        status: &'a str,
    ) -> Self {
        let selected = if filtered.is_empty() {
            0
        } else {
            selected.min(filtered.len() - 1)
        };
        Self {
            search,
            models,
            filtered,
            selected,
            status,
        }
    }
}

impl Widget for ModelPaletteWidget<'_> {
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
        row = paint_search_row(buf, area, row, self.search);
        row = row.saturating_add(1);

        if self.filtered.is_empty() {
            row = paint_text_row(
                buf,
                area,
                row,
                "no matching models",
                Style::default().fg(DIM),
            );
        } else {
            row = paint_model_list(buf, area, row, self.models, self.filtered, self.selected);
        }

        let footer = if self.filtered.is_empty() {
            format!("(0/0){}", footer_suffix(self.status))
        } else {
            format!(
                "({}/{}){}",
                self.selected + 1,
                self.filtered.len(),
                footer_suffix(self.status)
            )
        };
        let _ = paint_text_row(buf, area, row, &footer, Style::default().fg(MUTED));
    }
}

fn footer_suffix(status: &str) -> String {
    if status.is_empty() {
        "  enter select  esc cancel".into()
    } else {
        format!("  {status}")
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

fn paint_search_row(buf: &mut Buffer, area: Rect, y: u16, search: &ComposerState) -> u16 {
    let row_layout = padded_input_layout(area);
    if row_layout.wrap_width == 0 {
        return y.saturating_add(1);
    }
    paint_input_editor(
        buf,
        row_layout,
        search,
        y,
        1,
        Style::default().fg(TEXT),
        Style::default().fg(TEXT),
    );
    y.saturating_add(1)
}

pub fn model_row_capacity(palette_height: u16) -> usize {
    palette_height.saturating_sub(FIXED_ROWS).max(1) as usize
}

fn paint_model_list(
    buf: &mut Buffer,
    area: Rect,
    mut y: u16,
    models: &[ResolvedModel],
    filtered: &[usize],
    selected: usize,
) -> u16 {
    let visible = filtered
        .len()
        .min(model_row_capacity(area.height))
        .min(MAX_VISIBLE_MODELS)
        .max(1);
    let scroll_top = scroll_top(selected, filtered.len(), visible);

    for (row, model_index) in filtered
        .iter()
        .skip(scroll_top)
        .take(visible)
        .enumerate()
    {
        let Some(model) = models.get(*model_index) else {
            continue;
        };
        paint_model_row(buf, area, y, model, scroll_top + row == selected);
        y = y.saturating_add(1);
    }
    y
}

fn paint_model_row(buf: &mut Buffer, area: Rect, y: u16, model: &ResolvedModel, selected: bool) {
    let marker = if selected { "→" } else { " " };
    let id_style = if selected {
        Style::default().fg(NAME_SELECTED)
    } else {
        Style::default().fg(NAME)
    };
    let tag = format!(" [{}]", model.metadata_provider_id);
    let mut x = area.x.saturating_add(2);
    render_clipped(buf, x, y, marker, area.width as usize, id_style);
    x = x.saturating_add(2);
    render_clipped(buf, x, y, &model.id, area.width as usize, id_style);
    x = x.saturating_add(model.id.len() as u16);
    render_clipped(
        buf,
        x,
        y,
        &tag,
        area.width.saturating_sub(x - area.x) as usize,
        Style::default().fg(MUTED),
    );
}

fn scroll_top(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible {
        return 0;
    }
    selected.saturating_sub(visible - 1).min(total - visible)
}

pub fn model_palette_height(filtered_count: usize, max_height: u16) -> u16 {
    if max_height < FIXED_ROWS + 1 {
        return 0;
    }
    let model_rows = filtered_count
        .clamp(1, MAX_VISIBLE_MODELS)
        .min(model_row_capacity(max_height)) as u16;
    (FIXED_ROWS + model_rows).min(max_height)
}

pub fn model_input_wrap_width(area_width: u16) -> usize {
    padded_input_layout(Rect::new(0, 0, area_width.max(1), 1))
        .wrap_width
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::{model_palette_height, model_row_capacity, scroll_top, ModelPaletteWidget};
    use crate::backend::opencode::types::{
        InterleavedFieldState, Modalities, ModelModality, ModelStatus, OpenCodePlan,
        OpenCodeProtocol, ResolvedModel, SdkPackage,
    };
    use crate::frontend::composer::ComposerState;
    use ratatui::{backend::TestBackend, Terminal};

    fn sample_model(id: &str, name: &str) -> ResolvedModel {
        ResolvedModel {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            id: id.into(),
            name: name.into(),
            sdk_package: SdkPackage::OpenAiCompatible,
            protocol: OpenCodeProtocol::OpenAiCompatible,
            context_limit: 200_000,
            explicit_input_limit: None,
            output_limit: 32_000,
            tool_call: true,
            reasoning: true,
            modalities: Modalities {
                input: vec![ModelModality::Text],
                output: vec![ModelModality::Text],
            },
            interleaved: InterleavedFieldState::Unsupported { supported: false },
            cost: None,
            status: ModelStatus::Active,
            reasoning_variants: Vec::new(),
        }
    }

    #[test]
    fn model_palette_renders_search_and_model_tags() {
        let models = vec![
            sample_model("glm-5.2", "GLM-5.2"),
            sample_model("gpt-5.6-luna", "GPT-5.6 Luna"),
        ];
        let filtered = vec![0, 1];
        let search = ComposerState::default();
        let height = model_palette_height(filtered.len(), 12);
        let widget = ModelPaletteWidget::new(&search, &models, &filtered, 0, "");
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
        assert!(row(1).contains('>'));
        assert!(row(2).trim().is_empty());
        assert!(row(3).contains("glm-5.2"));
        let screen: String = (0..height).map(row).collect();
        assert!(screen.contains("glm-5.2 [opencode-go]"));
        assert!(screen.contains("gpt-5.6-luna [opencode-go]"));
    }

    #[test]
    fn model_palette_height_reserves_search_and_footer() {
        assert_eq!(model_palette_height(3, 12), 8);
        assert_eq!(model_palette_height(20, 12), 12);
        assert_eq!(model_palette_height(0, 0), 0);
    }

    #[test]
    fn scroll_top_keeps_selection_on_screen_with_six_visible_rows() {
        let visible = 6;
        let total = 20;
        assert_eq!(scroll_top(0, total, visible), 0);
        assert_eq!(scroll_top(5, total, visible), 0);
        assert_eq!(scroll_top(6, total, visible), 1);
        assert_eq!(scroll_top(13, total, visible), 8);
        assert_eq!(scroll_top(14, total, visible), 9);
        assert_eq!(scroll_top(19, total, visible), 14);
    }

    #[test]
    fn scrolling_down_keeps_highlight_on_the_last_visible_row() {
        let filtered: Vec<_> = (0..20).collect();
        let height = model_palette_height(filtered.len(), 12);
        let visible = model_row_capacity(height);
        assert_eq!(visible, 7);

        for selected in 0..20 {
            let top = scroll_top(selected, filtered.len(), visible);
            let arrow_row = selected - top;
            if selected < visible {
                assert_eq!(arrow_row, selected);
            } else if selected < filtered.len() - visible {
                assert_eq!(
                    arrow_row,
                    visible - 1,
                    "selection should stay on the last visible row at index {selected}"
                );
            } else {
                assert!(
                    arrow_row >= visible - 1,
                    "selection should not jump above the scroll window at index {selected}"
                );
            }
        }
    }
}
