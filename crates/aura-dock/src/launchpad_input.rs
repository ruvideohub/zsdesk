//! Input and asynchronous-search guards shared by the Launchpad and its tests.

/// Decode UTF-16 WM_CHAR units; Windows sends supplementary characters in two messages.
#[derive(Default)]
pub struct Utf16Input {
    high: Option<u16>,
}

impl Utf16Input {
    pub fn reset(&mut self) {
        self.high = None;
    }
    pub fn push(&mut self, unit: u16) -> Option<char> {
        match unit {
            0xD800..=0xDBFF => {
                self.high = Some(unit);
                None
            }
            0xDC00..=0xDFFF => {
                let high = self.high.take()? as u32 - 0xD800;
                let low = unit as u32 - 0xDC00;
                char::from_u32(0x10000 + (high << 10) + low)
            }
            _ => {
                self.high = None;
                char::from_u32(unit as u32)
            }
        }
    }
}

/// Single-line query paste, with a bound to avoid processing enormous clipboard text.
pub fn paste_query(query: &mut String, text: &str) -> bool {
    const MAX_CHARS: usize = 1024;
    let mut remaining = MAX_CHARS.saturating_sub(query.chars().count());
    let before = query.len();
    for ch in text.chars() {
        if remaining == 0 {
            break;
        }
        let ch = if ch.is_control() {
            if ch.is_whitespace() {
                ' '
            } else {
                continue;
            }
        } else {
            ch
        };
        query.push(ch);
        remaining -= 1;
    }
    query.len() != before
}

/// A CF_UNICODETEXT allocation must contain a NUL within the bounded allocation.
pub fn clipboard_text(words: &[u16]) -> Option<String> {
    let end = words.iter().position(|&w| w == 0)?;
    Some(String::from_utf16_lossy(&words[..end]))
}

/// Invalidate in-flight file searches immediately when input changes or closes.
#[derive(Default)]
pub struct SearchEpoch(u64);
impl SearchEpoch {
    pub fn invalidate(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
    pub fn current(&self) -> u64 {
        self.0
    }
    pub fn accepts(&self, generation: u64, requested: &str, current: &str, open: bool) -> bool {
        open && generation == self.0 && !requested.is_empty() && requested == current.trim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_pairs_and_invalid_sequences() {
        let mut d = Utf16Input::default();
        assert_eq!(d.push(0xD83D), None);
        assert_eq!(d.push(0xDE80), Some('🚀'));
        assert_eq!(d.push(0xDE80), None);
        assert_eq!(d.push(0xD83D), None);
        assert_eq!(d.push('я' as u16), Some('я'));
        assert_eq!(d.push(0xDE80), None);
        d.push(0xD83D);
        d.reset();
        assert_eq!(d.push(0xDE80), None);
    }
    #[test]
    fn paste_is_single_line_bounded_and_unicode_safe() {
        let mut q = "док ".to_string();
        assert!(paste_query(&mut q, "🚀\r\nтест\t\u{1}ok"));
        assert_eq!(q, "док 🚀  тест ok");
        assert!(!paste_query(&mut q, "\u{1}"));
        let mut full = "я".repeat(1023);
        assert!(paste_query(&mut full, "🚀tail"));
        assert_eq!(full.chars().count(), 1024);
        assert!(full.ends_with('🚀'));
        assert!(!paste_query(&mut full, "more"));
    }
    #[test]
    fn clipboard_requires_nul_inside_supplied_words() {
        assert_eq!(
            clipboard_text(&[0x410, 0xD83D, 0xDE80, 0, 0x42]),
            Some("А🚀".into())
        );
        assert_eq!(clipboard_text(&[0x410]), None);
        assert_eq!(clipboard_text(&[]), None);
        assert_eq!(clipboard_text(&[0]), Some(String::new()));
    }
    #[test]
    fn stale_replies_cannot_cross_edits_close_or_new_search() {
        let mut e = SearchEpoch::default();
        e.invalidate();
        let old = e.current();
        assert!(e.accepts(old, "one", " one ", true));
        e.invalidate(); // next keystroke, before the debounce timer fires
        assert!(!e.accepts(old, "one", "one two", true));
        assert!(!e.accepts(old, "one", "one", true));
        let current = e.current();
        assert!(!e.accepts(current, "one two", "", true)); // settings/gallery cleared query
        assert!(!e.accepts(current, "one two", "one two", false));
        e.invalidate(); // close and reopen, even with same query
        assert!(!e.accepts(current, "one two", "one two", true));
        assert!(!e.accepts(e.current(), "", "", true));
    }
}
