use regex::Regex;
use serde_json::Value;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageTokenError {
    Network(String),
    NotFound,
    Parse(String),
}

impl std::fmt::Display for PageTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageTokenError::Network(detail) => write!(f, "页面请求失败: {}", detail),
            PageTokenError::NotFound => write!(f, "页面中未找到 token"),
            PageTokenError::Parse(detail) => write!(f, "token 解析失败: {}", detail),
        }
    }
}

pub struct TokenRule {
    /// 页面 HTML 里需要匹配的正则，捕获组 1 就是 token
    pub regex: &'static str,
    /// 如果是 JSON 字符串还需要二次解析（微博 config 用）
    pub json_path: Option<&'static str>,
}

pub async fn fetch_token(
    client: &reqwest::Client,
    url: &str,
    rule: &TokenRule,
) -> Result<String, PageTokenError> {
    fetch_token_with_cookie(client, url, None, rule).await
}

pub async fn fetch_token_with_cookie(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
    rule: &TokenRule,
) -> Result<String, PageTokenError> {
    let mut request = client.get(url).header("User-Agent", USER_AGENT);
    if let Some(cookie) = cookie {
        let cookie = cookie.trim();
        if !cookie.is_empty() {
            request = request.header("Cookie", cookie);
        }
    }

    let response = request
        .send()
        .await
        .map_err(|error| PageTokenError::Network(error.to_string()))?;

    let status = response.status();
    if status != reqwest::StatusCode::OK {
        return Err(PageTokenError::Network(format!(
            "状态码 {}",
            status.as_u16()
        )));
    }

    let html = response
        .text()
        .await
        .map_err(|error| PageTokenError::Network(error.to_string()))?;

    extract_from_html(&html, rule)
}

pub fn extract_from_html(html: &str, rule: &TokenRule) -> Result<String, PageTokenError> {
    let regex =
        Regex::new(rule.regex).map_err(|error| PageTokenError::Parse(format!("正则无效: {}", error)))?;

    let captured = regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(PageTokenError::NotFound)?;

    match rule.json_path {
        None => Ok(captured),
        Some(path) => {
            let normalized = normalize_json_literal(&captured);
            let value: Value = serde_json::from_str(&normalized)
                .map_err(|error| PageTokenError::Parse(error.to_string()))?;
            let selected = get_by_dot_path(&value, path).ok_or(PageTokenError::NotFound)?;
            match selected {
                Value::String(text) => Ok(text.clone()),
                Value::Number(_) | Value::Bool(_) => Ok(selected.to_string()),
                _ => Err(PageTokenError::Parse(format!(
                    "JSON 路径 {} 指向的不是标量值",
                    path
                ))),
            }
        }
    }
}

/// 微博把编辑器配置写在 `JSON.parse('...')` 里，页面既可能输出 HTML 实体转义
/// （`&quot;` / `&#34;`），也可能输出反斜杠转义（`\"`）。这里只做最小还原：
/// 先解 HTML 实体，再解反斜杠转义，避免实体里的引号被反斜杠规则二次处理。
fn normalize_json_literal(raw: &str) -> String {
    let html_unescaped = raw.replace("&quot;", "\"").replace("&#34;", "\"");
    html_unescaped.replace("\\\"", "\"")
}

fn get_by_dot_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WECHAT_RULE: TokenRule = TokenRule {
        regex: r#"t:\s*"([^"]+)""#,
        json_path: None,
    };

    const WEIBO_RULE: TokenRule = TokenRule {
        regex: r#"config:\s*JSON\.parse\('(.+?)'\)"#,
        json_path: Some("token"),
    };

    #[test]
    fn extract_wechat_token() {
        let html = r#"<script>var cfg={t:"abc123"||""};</script>"#;
        assert_eq!(extract_from_html(html, &WECHAT_RULE).unwrap(), "abc123");
    }

    #[test]
    fn extract_weibo_token_from_json() {
        let html = r#"<script>var cfg = {config:JSON.parse('{"token":"xyz789"}')};</script>"#;
        assert_eq!(extract_from_html(html, &WEIBO_RULE).unwrap(), "xyz789");
    }

    #[test]
    fn regex_not_found() {
        let html = "<html><body>nothing here</body></html>";
        assert_eq!(
            extract_from_html(html, &WECHAT_RULE).unwrap_err(),
            PageTokenError::NotFound
        );
    }

    #[test]
    fn invalid_json() {
        let html = r#"<script>var cfg = {config:JSON.parse('not-a-json')};</script>"#;
        assert!(matches!(
            extract_from_html(html, &WEIBO_RULE).unwrap_err(),
            PageTokenError::Parse(_)
        ));
    }

    #[test]
    fn empty_capture_is_not_found() {
        let html = r#"<script>var cfg={t:""};</script>"#;
        assert_eq!(
            extract_from_html(html, &WECHAT_RULE).unwrap_err(),
            PageTokenError::NotFound
        );
    }

    #[test]
    fn missing_json_path_is_not_found() {
        let html = r#"<script>var cfg = {config:JSON.parse('{"other":"value"}')};</script>"#;
        assert_eq!(
            extract_from_html(html, &WEIBO_RULE).unwrap_err(),
            PageTokenError::NotFound
        );
    }

    #[test]
    fn html_entity_escaped_json_is_normalized() {
        let html =
            r#"<script>var cfg = {config:JSON.parse('{&quot;token&quot;:&quot;esc&quot;}')};</script>"#;
        assert_eq!(extract_from_html(html, &WEIBO_RULE).unwrap(), "esc");
    }

    #[test]
    fn backslash_escaped_json_is_normalized() {
        let html = r#"<script>var cfg = {config:JSON.parse('{\"token\":\"esc2\"}')};</script>"#;
        assert_eq!(extract_from_html(html, &WEIBO_RULE).unwrap(), "esc2");
    }

    #[test]
    fn dot_path_walks_nested_objects() {
        let html = r#"<script>var cfg = {config:JSON.parse('{"data":{"token":"nested"}}')};</script>"#;
        let rule = TokenRule {
            regex: WEIBO_RULE.regex,
            json_path: Some("data.token"),
        };
        assert_eq!(extract_from_html(html, &rule).unwrap(), "nested");
    }
}