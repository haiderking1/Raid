use crate::config::{AuthStore, ConnectProvider};
use crate::frontend::clip::render_clipped;
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

pub struct ConnectPaletteWidget {
    providers: &'static [ConnectProvider],
    selected: usize,
}

impl ConnectPaletteWidget {
    pub fn new(providers: &'static [ConnectProvider], selected: usize) -> Self {
        let selected = if providers.is_empty() {
            0
        } else {
            selected.min(providers.len() - 1)
        };
        Self {
            providers,
            selected,
        }
    }
}

impl Widget for ConnectPaletteWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let has_footer = area.height >= 2;
        let item_rows = if has_footer {
            (area.height - 1) as usize
        } else {
            area.height as usize
        }
        .max(1);

        let scroll_top = scroll_top(self.selected, self.providers.len(), item_rows);
        let name_col = self
            .providers
            .iter()
            .map(|provider| Line::from(provider.label).width())
            .max()
            .unwrap_or(0);

        for (row, provider) in self
            .providers
            .iter()
            .skip(scroll_top)
            .take(item_rows)
            .enumerate()
        {
            let y = area.y + row as u16;
            let selected = scroll_top + row == self.selected;
            render_provider_row(buf, area, y, provider, selected, name_col);
        }

        if has_footer {
            let footer = format!(
                "({}/{})  enter connect  esc cancel",
                self.selected + 1,
                self.providers.len().max(1)
            );
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

fn render_provider_row(
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
    let detail = format!("{status}");
    let mut x = area.x;
    render_clipped(buf, x, y, marker, area.width as usize, name_style);
    x = x.saturating_add(2);
    render_clipped(buf, x, y, provider.label, area.width as usize, name_style);
    x = x.saturating_add(name_col as u16 + 1);
    render_clipped(buf, x, y, "·", 2, Style::default().fg(DESCRIPTION));
    x = x.saturating_add(2);
    render_clipped(
        buf,
        x,
        y,
        &detail,
        area.width.saturating_sub(x - area.x) as usize,
        Style::default().fg(DESCRIPTION),
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

pub fn palette_height(provider_count: usize, max_height: u16) -> u16 {
    if provider_count == 0 || max_height < 2 {
        return 0;
    }
    let rows = provider_count.min(6).max(1) as u16;
    (rows + 1).min(max_height.max(2))
}
