/// Truncate a string at a char boundary, appending "…" if truncated.
pub fn truncate_str_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_str_safe("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate_str_safe("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate_str_safe("hello world", 5);
        assert_eq!(result, "hello…");
    }

    #[test]
    fn truncate_on_multibyte_boundary() {
        // Chinese chars are 3 bytes each: "你好世界" = 12 bytes
        let result = truncate_str_safe("你好世界", 7);
        // Can't cut at byte 7 (mid-char), backs up to byte 6 (after "你好")
        assert_eq!(result, "你好…");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_str_safe("", 5), "");
    }

    #[test]
    fn truncate_at_zero() {
        assert_eq!(truncate_str_safe("hello", 0), "…");
    }
}
