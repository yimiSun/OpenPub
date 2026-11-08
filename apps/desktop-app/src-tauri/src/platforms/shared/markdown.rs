pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn to_html_content(markdown: &str) -> String {
    let escaped = escape_html(markdown);
    let mut html = String::new();
    for line in escaped.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        html.push_str("<p>");
        html.push_str(trimmed);
        html.push_str("</p>");
    }
    if html.is_empty() {
        html.push_str("<p></p>");
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_special_characters() {
        assert_eq!(to_html_content("a < b & c"), "<p>a &lt; b &amp; c</p>");
    }

    #[test]
    fn wraps_each_non_empty_line_in_paragraph() {
        let html = to_html_content("第一行\n\n第二行\n第三行");
        assert_eq!(html, "<p>第一行</p><p>第二行</p><p>第三行</p>");
    }

    #[test]
    fn empty_source_yields_single_empty_paragraph() {
        assert_eq!(to_html_content(""), "<p></p>");
        assert_eq!(to_html_content("   \n\n  "), "<p></p>");
    }

    #[test]
    fn conversion_is_stable_for_repeated_calls() {
        let source = "# 标题\n\n- 列表";
        assert_eq!(to_html_content(source), to_html_content(source));
    }
}