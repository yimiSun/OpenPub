use std::sync::OnceLock;

use comrak::{markdown_to_html, Options};
use regex::Regex;

fn comrak_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.render.r#unsafe = true;
    options.render.hardbreaks = true;
    options
}

pub fn to_html_content(markdown: &str) -> String {
    if markdown.trim().is_empty() {
        return "<p></p>".to_string();
    }

    let html = markdown_to_html(markdown, &comrak_options());
    let trimmed = html.trim();
    if trimmed.is_empty() {
        "<p></p>".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 知乎专属 HTML：Draft.js 编辑器只可靠地识别基础块级标签。
///
/// 知乎正文存放在 Draft.js 的 Immutable 数据结构里，`<table>` / `<pre><code>`
/// 这类结构标签在粘贴或 `insertHTML` 时会被解析失败并整段丢弃，最终只剩零散
/// 的 `<p>`。因此这里在通用 HTML 之上做两层降级：
/// - 代码块 → `<blockquote>` + `<br />` 换行，知乎对引用块兼容性最好；
/// - 表格 → 用 ` | ` 连接的纯文本行，原生表格基本必丢。
///
/// 最后再过一遍标签白名单，只保留 Draft.js 解析成功率最高的标签。
pub fn to_zhihu_html_content(markdown: &str) -> String {
    let html = to_html_content(markdown);
    let html = flatten_code_blocks(&html);
    let html = flatten_tables(&html);
    sanitize_tags(&html)
}

/// 允许出现在知乎正文里的标签。`li` 是列表的必要子节点，`code` 保留行内代码。
const ZHIHU_ALLOWED_TAGS: &[&str] = &[
    "p",
    "br",
    "strong",
    "b",
    "em",
    "i",
    "a",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "code",
    "hr",
];

fn code_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"<pre(?:\s[^>]*)?><code(?:\s[^>]*)?>([\s\S]*?)</code></pre>")
            .expect("代码块正则非法")
    })
}

fn table_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<table(?:\s[^>]*)?>[\s\S]*?</table>").expect("表格正则非法"))
}

fn table_row_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<tr(?:\s[^>]*)?>[\s\S]*?</tr>").expect("表格行正则非法"))
}

fn table_cell_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"<t[hd](?:\s[^>]*)?>[\s\S]*?</t[hd]>").expect("表格单元格正则非法")
    })
}

fn tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<(/)?([a-zA-Z][a-zA-Z0-9]*)([^>]*)>").expect("标签正则非法"))
}

fn href_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"href\s*=\s*"([^"]*)""#).expect("链接正则非法"))
}

/// 代码块降级为引用块：`<pre><code>` 换成 `<blockquote>`，换行用 `<br />` 维系。
fn flatten_code_blocks(html: &str) -> String {
    code_block_regex()
        .replace_all(html, |caps: &regex::Captures| {
            let code = caps.get(1).map_or("", |m| m.as_str()).trim_end_matches('\n');
            if code.is_empty() {
                return String::new();
            }
            let body = code.split('\n').collect::<Vec<_>>().join("<br />");
            format!("<blockquote><p>{}</p></blockquote>", body)
        })
        .into_owned()
}

/// 表格扁平化为纯文本：每行单元格用 ` | ` 连接，行之间用 `<br />` 分隔。
fn flatten_tables(html: &str) -> String {
    table_regex()
        .replace_all(html, |caps: &regex::Captures| {
            let text = table_to_text(&caps[0]);
            if text.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", text)
            }
        })
        .into_owned()
}

fn table_to_text(table: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for row in table_row_regex().captures_iter(table) {
        let cells: Vec<String> = table_cell_regex()
            .captures_iter(&row[0])
            .map(|cell| tag_regex().replace_all(&cell[0], "").trim().to_string())
            .collect();
        // 全空行（分隔线或空行）没有信息量，直接跳过。
        if cells.iter().any(|cell| !cell.is_empty()) {
            lines.push(cells.join(" | "));
        }
    }
    lines.join("<br />")
}

/// 标签白名单：不在名单里的标签整体剥掉（保留其文本内容）。
fn sanitize_tags(html: &str) -> String {
    tag_regex()
        .replace_all(html, |caps: &regex::Captures| {
            // `(/)?` 是可选组：开口标签时它不参与匹配，必须用 get 而不是索引。
            let closing = caps.get(1).is_some();
            let name = caps[2].to_ascii_lowercase();
            if !ZHIHU_ALLOWED_TAGS.contains(&name.as_str()) {
                return String::new();
            }
            if closing {
                return format!("</{}>", name);
            }
            if name == "a" {
                // 只保留 href，其余 class / title 等属性一律丢弃。
                let attrs = caps.get(3).map_or("", |m| m.as_str());
                return match href_regex().captures(attrs) {
                    Some(href) => format!("<a href=\"{}\">", &href[1]),
                    None => "<a>".to_string(),
                };
            }
            if name == "br" || name == "hr" {
                return format!("<{} />", name);
            }
            format!("<{}>", name)
        })
        .into_owned()
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
        let html = to_html_content("第一行\n\n第二行");
        assert!(html.contains("<p>第一行</p>"), "实际输出: {}", html);
        assert!(html.contains("<p>第二行</p>"), "实际输出: {}", html);
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

    #[test]
    fn renders_fenced_code_block() {
        let html = to_html_content("```rust\nfn main() {}\n```");
        assert!(
            html.contains("<pre><code class=\"language-rust\">"),
            "实际输出: {}",
            html
        );
        assert!(html.contains("fn main() {}"), "实际输出: {}", html);
        assert!(!html.contains("```"), "实际输出: {}", html);
    }

    #[test]
    fn renders_lists_as_list_markup() {
        let html = to_html_content("- a\n- b");
        assert!(html.contains("<ul>"), "实际输出: {}", html);
        assert!(html.contains("<li>a</li>"), "实际输出: {}", html);
        assert!(html.contains("<li>b</li>"), "实际输出: {}", html);
    }

    #[test]
    fn renders_tables_as_table_markup() {
        let html = to_html_content("| 列1 | 列2 |\n| --- | --- |\n| a | b |");
        assert!(html.contains("<table>"), "实际输出: {}", html);
        assert!(html.contains("<thead>"), "实际输出: {}", html);
        assert!(html.contains("<td>a</td>"), "实际输出: {}", html);
    }

    #[test]
    fn single_newline_becomes_hard_break() {
        let html = to_html_content("第一行\n第二行");
        assert!(html.contains("<br />"), "实际输出: {}", html);
        assert!(!html.contains("第一行第二行"), "实际输出: {}", html);
    }

    #[test]
    fn raw_html_tags_pass_through() {
        let html = to_html_content("<div>原样保留</div>");
        assert!(html.contains("<div>原样保留</div>"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_downgrades_code_blocks_to_blockquote() {
        let html = to_zhihu_html_content("```rust\nfn main() {}\n```");
        assert!(html.contains("<blockquote>"), "实际输出: {}", html);
        assert!(html.contains("fn main() {}"), "实际输出: {}", html);
        // Draft.js 解析不了的 pre/code 结构必须彻底消失。
        assert!(!html.contains("<pre"), "实际输出: {}", html);
        assert!(!html.contains("<code"), "实际输出: {}", html);
        assert!(!html.contains("language-rust"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_code_block_keeps_line_breaks() {
        let html = to_zhihu_html_content("```\nlet a = 1;\nlet b = 2;\n```");
        assert!(html.contains("let a = 1;"), "实际输出: {}", html);
        assert!(html.contains("let b = 2;"), "实际输出: {}", html);
        assert!(html.contains("<br />"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_flattens_tables_into_text_lines() {
        let html = to_zhihu_html_content("| 列1 | 列2 |\n| --- | --- |\n| a | b |");
        assert!(html.contains("列1 | 列2"), "实际输出: {}", html);
        assert!(html.contains("a | b"), "实际输出: {}", html);
        // 原生表格必丢，输出里不能再出现表格标签。
        assert!(!html.contains("<table"), "实际输出: {}", html);
        assert!(!html.contains("<tr"), "实际输出: {}", html);
        assert!(!html.contains("<td"), "实际输出: {}", html);
        assert!(!html.contains("<thead"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_keeps_only_compatible_tags() {
        let html = to_zhihu_html_content(
            "# 标题\n\n**粗体** 与 *斜体*\n\n- 列表项\n\n1. 有序项\n\n> 引用\n\n[链接](https://example.com)",
        );
        for tag in ["<h1>", "<strong>", "<em>", "<ul>", "<li>", "<ol>", "<blockquote>"] {
            assert!(html.contains(tag), "缺少 {}，实际输出: {}", tag, html);
        }
        assert!(html.contains("<a href=\"https://example.com\">"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_strips_unsupported_tags_but_keeps_text() {
        let html = to_zhihu_html_content("<div>原样保留</div>\n\n<span>行内文本</span>");
        assert!(html.contains("原样保留"), "实际输出: {}", html);
        assert!(html.contains("行内文本"), "实际输出: {}", html);
        assert!(!html.contains("<div>"), "实际输出: {}", html);
        assert!(!html.contains("<span"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_drops_image_tags() {
        let html = to_zhihu_html_content("![配图](https://example.com/a.png)");
        assert!(!html.contains("<img"), "实际输出: {}", html);
    }

    #[test]
    fn zhihu_escapes_special_characters() {
        assert_eq!(
            to_zhihu_html_content("a < b & c"),
            "<p>a &lt; b &amp; c</p>"
        );
    }

    #[test]
    fn zhihu_empty_source_yields_single_empty_paragraph() {
        assert_eq!(to_zhihu_html_content(""), "<p></p>");
        assert_eq!(to_zhihu_html_content("   \n\n  "), "<p></p>");
    }
}