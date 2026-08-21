#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSseEvent {
    pub event: String,
    pub data: String,
    pub id: Option<String>,
}

pub struct SseParser {
    carry: String,
    event_name: String,
    data_lines: Vec<String>,
    last_id: Option<String>,
    saw_bom: bool,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            carry: String::new(),
            event_name: String::new(),
            data_lines: Vec::new(),
            last_id: None,
            saw_bom: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<ParsedSseEvent> {
        let text = String::from_utf8_lossy(chunk);
        self.consume(&text)
    }

    pub fn finish(&mut self) -> Vec<ParsedSseEvent> {
        let mut events = Vec::new();
        if !self.carry.is_empty() {
            let trailing_line = if self.carry.ends_with('\r') {
                self.carry[..self.carry.len() - 1].to_string()
            } else {
                self.carry.clone()
            };
            events.extend(self.handle_line(&trailing_line));
            self.carry.clear();
        }
        if let Some(event) = self.dispatch() {
            events.push(event);
        }
        events
    }

    fn consume(&mut self, text: &str) -> Vec<ParsedSseEvent> {
        let mut text = text;
        if !self.saw_bom && !text.is_empty() {
            if text.starts_with('\u{feff}') {
                text = &text[1..];
            }
            self.saw_bom = true;
        }

        let mut events = Vec::new();
        let mut buffer = String::new();
        buffer.push_str(&self.carry);
        buffer.push_str(text);
        self.carry.clear();

        let mut index = 0;
        let bytes = buffer.as_bytes();
        while index < bytes.len() {
            let cr = buffer[index..].find('\r');
            let lf = buffer[index..].find('\n');
            let (newline, skip) = match (cr, lf) {
                (None, None) => break,
                (None, Some(lf)) => (index + lf, 1),
                (Some(cr), None) => {
                    if index + cr + 1 == bytes.len() {
                        break;
                    }
                    if bytes.get(index + cr + 1) == Some(&b'\n') {
                        (index + cr, 2)
                    } else {
                        (index + cr, 1)
                    }
                }
                (Some(cr), Some(lf)) if cr <= lf => {
                    if index + cr + 1 == bytes.len() {
                        break;
                    }
                    if bytes.get(index + cr + 1) == Some(&b'\n') {
                        (index + cr, 2)
                    } else {
                        (index + cr, 1)
                    }
                }
                (_, Some(lf)) => (index + lf, 1),
            };

            events.extend(self.handle_line(&buffer[index..newline]));
            index = newline + skip;
        }

        self.carry = buffer[index..].to_string();
        events
    }

    fn handle_line(&mut self, line: &str) -> Vec<ParsedSseEvent> {
        if line.is_empty() {
            return self.dispatch().into_iter().collect();
        }
        if line.starts_with(':') {
            return Vec::new();
        }

        let (field, mut value) = match line.find(':') {
            Some(colon) => (&line[..colon], line[colon + 1..].to_string()),
            None => (line, String::new()),
        };
        if value.starts_with(' ') {
            value = value[1..].to_string();
        }

        match field {
            "event" => self.event_name = value,
            "data" => self.data_lines.push(value),
            "id" if !value.contains('\0') => self.last_id = Some(value),
            _ => {}
        }
        Vec::new()
    }

    fn dispatch(&mut self) -> Option<ParsedSseEvent> {
        if self.event_name.is_empty() && self.data_lines.is_empty() {
            self.reset_fields();
            return None;
        }
        let data = self.data_lines.join("\n");
        let event = ParsedSseEvent {
            event: std::mem::take(&mut self.event_name),
            data,
            id: self.last_id.clone(),
        };
        self.reset_fields();
        Some(event)
    }

    fn reset_fields(&mut self) {
        self.event_name.clear();
        self.data_lines.clear();
    }
}

pub fn is_sse_terminal_sentinel(event: &ParsedSseEvent) -> bool {
    event.data == "[DONE]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_data_and_done_sentinel() {
        let mut parser = SseParser::new();
        let events = parser.push(b"event: message\ndata: {\"a\":1}\n\ndata: [DONE]\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "message");
        assert_eq!(events[0].data, "{\"a\":1}");
        assert!(is_sse_terminal_sentinel(&events[1]));
    }
}
