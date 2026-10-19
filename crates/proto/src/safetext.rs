//! One-line labels from untrusted text. Port of SafeText.swift:17-44.
//!
//! Deviation: `limit` counts `char`s (Unicode scalars), not grapheme clusters, and the removed
//! set is an explicit list of format, private-use and similar code points rather than Unicode
//! general-category tables.

pub const LABEL_LIMIT: usize = 64;

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{2028}' || c == '\u{2029}'
}

fn is_removed(c: char) -> bool {
    let u = u32::from(c);
    c.is_control()
        || matches!(
            u,
            0xAD | 0x600..=0x605
                | 0x61C
                | 0x6DD
                | 0x70F
                | 0x180E
                | 0x200B..=0x200F
                | 0x202A..=0x202E
                | 0x2060..=0x2064
                | 0x2066..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0xE000..=0xF8FF
                | 0xF0000..=0x10FFFF
                | 0xE0001
                | 0xE0020..=0xE007F
        )
}

/// `text` as one line: whitespace runs become one space, unprintable characters are removed, the
/// ends are trimmed and at most `limit` chars are kept.
pub fn label(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for c in text.chars() {
        if is_space(c) {
            pending_space = !out.is_empty();
            continue;
        }
        if is_removed(c) {
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    if out.chars().count() > limit {
        out = out.chars().take(limit).collect();
        while out.ends_with(' ') {
            out.pop();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace() {
        assert_eq!(label("  a\n\tb  ", 64), "a b");
    }

    #[test]
    fn removes_bidi() {
        assert_eq!(label("\u{202E}abc", 64), "abc");
    }

    #[test]
    fn limits_chars() {
        assert_eq!(label(&"x".repeat(100), LABEL_LIMIT).chars().count(), 64);
    }

    #[test]
    fn zero_width_is_empty() {
        assert_eq!(label("\u{200B}", 64), "");
    }
}
