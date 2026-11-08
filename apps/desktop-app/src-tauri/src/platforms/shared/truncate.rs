pub fn truncate_chars(input: &str, limit: usize) -> String {
    if input.chars().count() <= limit {
        input.to_string()
    } else {
        let mut truncated: String = input.chars().take(limit).collect();
        truncated.push_str("...(已截断)");
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_multibyte_text_without_panic() {
        let long = "中".repeat(50);
        let truncated = truncate_chars(&long, 10);
        assert!(truncated.starts_with(&"中".repeat(10)));
        assert!(truncated.ends_with("...(已截断)"));
        assert_eq!(truncated.chars().filter(|c| *c == '中').count(), 10);
    }

    #[test]
    fn returns_short_text_unchanged() {
        let short = "短文本";
        assert_eq!(truncate_chars(short, 10), short);
    }

    #[test]
    fn exact_length_is_not_marked_as_truncated() {
        let exact = "abcde";
        assert_eq!(truncate_chars(exact, 5), exact);
    }
}