mod theme;

#[cfg(test)]
mod tests;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;

pub fn render(markdown: &str, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut renderer = Renderer::new(width);
    for event in Parser::new_ext(markdown, options) {
        renderer.handle(event);
    }
    renderer.finish()
}

struct ListState {
    ordered: bool,
    index: u64,
    indent: usize,
}

struct Renderer {
    width: usize,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    line_width: usize,
    strong: u8,
    emphasis: u8,
    strikethrough: u8,
    in_code: bool,
    in_link: bool,
    lists: Vec<ListState>,
    pending_marker: Option<String>,
    in_code_block: bool,
    blockquote: u8,
    heading_level: Option<u8>,
    hanging: usize,
}

impl Renderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            spans: Vec::new(),
            line_width: 0,
            strong: 0,
            emphasis: 0,
            strikethrough: 0,
            in_code: false,
            in_link: false,
            lists: Vec::new(),
            pending_marker: None,
            in_code_block: false,
            blockquote: 0,
            heading_level: None,
            hanging: 0,
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                self.in_code = true;
                self.text(&code);
                self.in_code = false;
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.flush_paragraph();
                self.lines.push(Line::from(Span::styled(
                    "─".repeat(self.width.min(40)),
                    theme::rule(),
                )));
            }
            Event::TaskListMarker(done) => {
                let marker = if done { "[x] " } else { "[ ] " };
                self.pending_marker = Some(match self.pending_marker.take() {
                    Some(prefix) => format!("{prefix}{marker}"),
                    None => marker.to_owned(),
                });
            }
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::FootnoteReference(reference) => self.text(&format!("[^{reference}]")),
            Event::InlineMath(math) | Event::DisplayMath(math) => self.text(&math),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_paragraph();
                self.heading_level = Some(level as u8);
                self.pending_marker = Some(heading_marker(level));
            }
            Tag::BlockQuote(_) => {
                self.flush_paragraph();
                self.blockquote = self.blockquote.saturating_add(1);
            }
            Tag::CodeBlock(kind) => {
                self.flush_paragraph();
                self.flush_marker();
                if self.line_width > 0 {
                    self.flush_line();
                }
                self.in_code_block = true;
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    self.push_raw(format!("  {lang}"), theme::quote());
                    self.flush_line();
                }
            }
            Tag::List(start) => {
                self.flush_paragraph();
                let indent = self.lists.len() * 2;
                self.lists.push(ListState {
                    ordered: start.is_some(),
                    index: start.unwrap_or(1),
                    indent,
                });
            }
            Tag::Item => {
                if let Some(list) = self.lists.last_mut() {
                    let marker = if list.ordered {
                        let item = format!("{:width$}. ", list.index, width = 1);
                        list.index += 1;
                        format!("{}{item}", " ".repeat(list.indent))
                    } else {
                        format!("{}• ", " ".repeat(list.indent))
                    };
                    self.pending_marker = Some(marker);
                }
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strikethrough += 1,
            Tag::Link { .. } => self.in_link = true,
            Tag::Image { .. } => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
                if self.lists.is_empty() {
                    self.hanging = 0;
                }
            }
            TagEnd::Heading(_) => {
                self.flush_paragraph();
                self.heading_level = None;
                self.hanging = 0;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph();
                self.blockquote = self.blockquote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.flush_line();
                self.in_code_block = false;
                self.flush_paragraph();
            }
            TagEnd::List(_) => {
                self.flush_paragraph();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush_line();
                self.hanging = 0;
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strikethrough = self.strikethrough.saturating_sub(1),
            TagEnd::Link => self.in_link = false,
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            for (index, line) in text.split('\n').enumerate() {
                if index > 0 {
                    self.flush_line();
                }
                if line.is_empty() && index == text.split('\n').count() - 1 {
                    continue;
                }
                self.push_span("│ ", theme::code_block());
                for grapheme in line.graphemes(true) {
                    let grapheme_width = display_width(grapheme);
                    if self.line_width > 0 && self.line_width + grapheme_width > self.width {
                        self.flush_line();
                        self.push_span("│ ", theme::code_block());
                    }
                    self.push_span(grapheme, theme::code_block());
                }
            }
            return;
        }

        let style = self.inline_style();
        for word in split_keep_spaces(text) {
            self.push_word(word, style);
        }
    }

    fn inline_style(&self) -> Style {
        if self.in_code {
            return theme::inline_code();
        }
        if self.in_link {
            return theme::link();
        }
        let mut style = if let Some(level) = self.heading_level {
            theme::heading(level)
        } else if self.emphasis > 0 {
            theme::emphasis()
        } else if self.strong > 0 {
            theme::strong()
        } else {
            theme::body()
        };
        if self.strong > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.strikethrough > 0 {
            style = style.patch(theme::strikethrough());
        }
        style
    }

    fn push_word(&mut self, word: &str, style: Style) {
        self.flush_marker();
        let indent = self.current_indent();
        if word == " " && self.line_width <= indent {
            return;
        }
        let word_width = display_width(word);
        if self.line_width > indent && self.line_width + word_width > self.width {
            self.flush_line();
            self.flush_marker();
        }
        if display_width(word) > self.width.saturating_sub(self.line_width) {
            for grapheme in word.graphemes(true) {
                let grapheme_width = display_width(grapheme);
                if self.line_width > 0 && self.line_width + grapheme_width > self.width {
                    self.flush_line();
                    self.flush_marker();
                }
                self.push_span(grapheme, style);
            }
            return;
        }
        self.push_span(word, style);
    }

    fn push_raw(&mut self, text: String, style: Style) {
        self.flush_marker();
        self.push_span(&text, style);
    }

    fn flush_marker(&mut self) {
        if self.line_width != 0 {
            return;
        }
        if self.blockquote > 0 {
            let quote = "▌ ".repeat(self.blockquote as usize);
            self.push_span(&quote, theme::quote());
        }
        if let Some(marker) = self.pending_marker.take() {
            let marker_style = if marker.contains('#') && marker.trim_start().starts_with('#') {
                theme::heading(marker.chars().take_while(|c| *c == '#').count() as u8)
            } else {
                theme::list_marker()
            };
            self.hanging = display_width(&marker);
            self.push_span(&marker, marker_style);
            return;
        }
        if self.hanging > 0 {
            self.push_span(&" ".repeat(self.hanging), theme::body());
        }
    }

    fn current_indent(&self) -> usize {
        self.quote_prefix() + self.hanging
    }

    fn quote_prefix(&self) -> usize {
        if self.blockquote == 0 {
            0
        } else {
            display_width(&"▌ ".repeat(self.blockquote as usize))
        }
    }

    fn push_span(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.line_width += display_width(text);
        if let Some(last) = self.spans.last_mut()
            && last.style == style
        {
            last.content.to_mut().push_str(text);
            return;
        }
        self.spans.push(Span::styled(text.to_owned(), style));
    }

    fn flush_line(&mut self) {
        if self.spans.is_empty() {
            self.line_width = 0;
            return;
        }
        self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        self.line_width = 0;
    }

    fn flush_paragraph(&mut self) {
        self.flush_line();
        if self.lines.last().is_some_and(|line| !line.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

fn heading_marker(level: HeadingLevel) -> String {
    let count = level as u8;
    format!("{} ", "#".repeat(count as usize))
}

fn display_width(text: &str) -> usize {
    Line::from(text).width()
}

fn split_keep_spaces(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with(' ') {
            let spaces = rest.find(|c: char| c != ' ').unwrap_or(rest.len());
            parts.push(&rest[..spaces]);
            rest = &rest[spaces..];
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            parts.push(&rest[..end]);
            rest = &rest[end..];
        }
    }
    parts
}
