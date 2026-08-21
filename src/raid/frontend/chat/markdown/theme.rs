use ratatui::style::{Color, Modifier, Style};

pub const BODY: Color = Color::Rgb(230, 230, 230);
pub const HEADING: Color = Color::Rgb(80, 196, 184);
pub const EMPHASIS: Color = Color::Rgb(212, 176, 120);
pub const CODE: Color = Color::Rgb(80, 196, 184);
pub const QUOTE: Color = Color::Rgb(130, 148, 150);
pub const RULE: Color = Color::Rgb(96, 96, 96);
pub const LINK: Color = Color::Rgb(80, 196, 184);

pub fn body() -> Style {
    Style::default().fg(BODY)
}

pub fn heading(level: u8) -> Style {
    let style = Style::default().fg(HEADING);
    if level <= 2 {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

pub fn strong() -> Style {
    body().add_modifier(Modifier::BOLD)
}

pub fn emphasis() -> Style {
    Style::default().fg(EMPHASIS).add_modifier(Modifier::ITALIC)
}

pub fn strikethrough() -> Style {
    body().add_modifier(Modifier::CROSSED_OUT)
}

pub fn inline_code() -> Style {
    Style::default().fg(CODE)
}

pub fn code_block() -> Style {
    Style::default().fg(Color::Rgb(180, 180, 180))
}

pub fn quote() -> Style {
    Style::default().fg(QUOTE)
}

pub fn rule() -> Style {
    Style::default().fg(RULE)
}

pub fn link() -> Style {
    Style::default().fg(LINK).add_modifier(Modifier::UNDERLINED)
}

pub fn list_marker() -> Style {
    Style::default().fg(EMPHASIS)
}
