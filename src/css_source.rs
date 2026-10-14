use crate::facts::SourceText;
use std::ops::Range;

/// Maps decoded CSS bytes back to the original Astro input. Multiple decoded
/// bytes may originate from the same entity or JavaScript escape sequence.
pub(crate) struct CssSource {
    pub code: String,
    origins: Vec<Range<u32>>,
    base: u32,
}
impl CssSource {
    pub fn direct(raw: &str, base: u32) -> Self {
        let mut origins = Vec::with_capacity(raw.len());
        for (i, ch) in raw.char_indices() {
            let span = base + i as u32..base + (i + ch.len_utf8()) as u32;
            for _ in 0..ch.len_utf8() {
                origins.push(span.clone());
            }
        }
        Self {
            code: raw.into(),
            origins,
            base,
        }
    }
    pub fn attribute(raw: &str, base: u32, javascript: bool) -> Self {
        let (body, start) = if raw.len() >= 2 && matches!(raw.as_bytes()[0], b'\'' | b'"' | b'`') {
            (&raw[1..raw.len() - 1], base + 1)
        } else {
            (raw, base)
        };
        let mut result = Self {
            code: String::new(),
            origins: Vec::new(),
            base: start,
        };
        let mut i = 0;
        while i < body.len() {
            if !javascript
                && body.as_bytes()[i] == b'&'
                && let Some(relative) = body[i..].find(';').filter(|end| *end <= 40)
            {
                let end = i + relative + 1;
                let entity = &body[i..end];
                let decoded = html_escape::decode_html_entities(entity);
                if decoded != entity {
                    result.push(&decoded, start + i as u32..start + end as u32);
                    i = end;
                    continue;
                }
            }
            if javascript && body.as_bytes()[i] == b'\\' {
                let begin = i;
                i += 1;
                if i == body.len() {
                    break;
                }
                let ch = body[i..].chars().next().unwrap();
                i += ch.len_utf8();
                let decoded = match ch {
                    'n' => "\n".into(),
                    'r' => "\r".into(),
                    't' => "\t".into(),
                    'b' => "\u{8}".into(),
                    'f' => "\u{c}".into(),
                    'v' => "\u{b}".into(),
                    '0' => "\0".into(),
                    '\n' | '\u{2028}' | '\u{2029}' => String::new(),
                    '\r' => {
                        if body.as_bytes().get(i) == Some(&b'\n') {
                            i += 1;
                        }
                        String::new()
                    }
                    'x' if body.len() >= i + 2 => {
                        let value = u32::from_str_radix(&body[i..i + 2], 16).unwrap_or(0xfffd);
                        i += 2;
                        char::from_u32(value).unwrap_or('\u{fffd}').to_string()
                    }
                    'u' => {
                        let value = if body.as_bytes().get(i) == Some(&b'{') {
                            i += 1;
                            let end = body[i..].find('}').map_or(body.len(), |n| i + n);
                            let value = u32::from_str_radix(&body[i..end], 16).unwrap_or(0xfffd);
                            i = (end + 1).min(body.len());
                            value
                        } else if body.len() >= i + 4 {
                            let high = u32::from_str_radix(&body[i..i + 4], 16).unwrap_or(0xfffd);
                            i += 4;
                            if (0xd800..=0xdbff).contains(&high)
                                && body.get(i..i + 2) == Some("\\u")
                                && body.len() >= i + 6
                            {
                                let low = u32::from_str_radix(&body[i + 2..i + 6], 16).unwrap_or(0);
                                if (0xdc00..=0xdfff).contains(&low) {
                                    i += 6;
                                    0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
                                } else {
                                    high
                                }
                            } else {
                                high
                            }
                        } else {
                            0xfffd
                        };
                        char::from_u32(value).unwrap_or('\u{fffd}').to_string()
                    }
                    _ => ch.to_string(),
                };
                result.push(&decoded, start + begin as u32..start + i as u32);
                continue;
            }
            let ch = body[i..].chars().next().unwrap();
            let end = i + ch.len_utf8();
            result.push(&body[i..end], start + i as u32..start + end as u32);
            i = end;
        }
        result
    }
    fn push(&mut self, text: &str, span: Range<u32>) {
        self.code.push_str(text);
        self.origins.extend((0..text.len()).map(|_| span.clone()));
    }
    pub fn range(&self, start: usize, end: usize) -> Range<u32> {
        if start == end {
            let offset = self.origins.get(start).map_or_else(
                || self.origins.last().map_or(self.base, |o| o.end),
                |o| o.start,
            );
            return offset..offset;
        }
        self.origins[start].start..self.origins[end - 1].end
    }
    pub fn source_text(&self, source: &str, start: usize, end: usize) -> SourceText {
        let span = self.range(start, end);
        SourceText {
            raw: source[span.start as usize..span.end as usize].into(),
            span,
        }
    }
    pub fn location(&self, line: u32, column: u32) -> usize {
        let mut current_line = 0;
        let mut current_column = 1;
        let mut after_cr = false;
        for (index, ch) in self.code.char_indices() {
            if current_line == line && current_column >= column {
                return index;
            }
            if ch == '\r' {
                current_line += 1;
                current_column = 1;
                after_cr = true;
            } else if ch == '\n' {
                if !after_cr {
                    current_line += 1;
                }
                current_column = 1;
                after_cr = false;
            } else {
                current_column += ch.len_utf16() as u32;
                after_cr = false;
            }
        }
        self.code.len()
    }
}
