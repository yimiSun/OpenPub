use regex::Regex;
use serde_json::Value;

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "baijiahao_config";
const PLATFORM: &str = "百家号";
const BASE_URL: &str = "https://baijiahao.baidu.com";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

/// 编辑页里可能承载发布凭证的 HTML 模式。
///
/// 百家号前端改版频繁，凭证的落地位置换过好几轮：早期是
/// `window.__BJH__INIT__AUTH__ = 'xxx'` 这种裸字符串赋值，后来变成
/// `window.__BJH__INIT__AUTH__ = {...}` 对象里的 `token` 字段，也出现过
/// `"token":"xxx"` 直接内联在初始化 JSON 里的写法。单条正则会随改版整条失效，
/// 因此这里按「从具体到宽泛」依次尝试。
const AUTH_TOKEN_PATTERNS: &[&str] = &[
    r#"window\.__BJH__INIT__AUTH__\s*=\s*['"]([^'"]+)['"]"#,
    r#"window\.__BJH__INIT__AUTH__\s*=\s*\{[^}]*?["']?token["']?\s*:\s*['"]([^'"]+)['"]"#,
    r#"window\.__INIT__AUTH__\s*=\s*\{[^}]*?["']?token["']?\s*:\s*['"]([^'"]+)['"]"#,
    r#""token"\s*:\s*"([^"]+)""#,
    r#""token"\s*:\s*'([^']+)'"#,
    r#"name=["']token["']\s+content=["']([^"']+)["']"#,
];

/// 响应头里可能带凭证的键名。服务端在渲染编辑页时会把 token 回写到头上，
/// 这条路径比解析 HTML 稳得多，是正则失效后的第一顺位兜底。
const AUTH_TOKEN_HEADERS: &[&str] = &["token", "x-token", "x-bjh-token", "bjh-token"];

fn extract_auth_token(html: &str) -> Option<String> {
    for pattern in AUTH_TOKEN_PATTERNS {
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(_) => continue,
        };
        if let Some(captures) = regex.captures(html) {
            if let Some(token) = captures.get(1) {
                let token = token.as_str().trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

/// 从跳转后的最终 URL 查询串里取 token（`...?type=news&token=xxx`）。
fn token_from_url(final_url: &str) -> Option<String> {
    let query = final_url.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        if key.trim() == "token" {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 三路取凭证：响应头 → 页面 HTML → 最终 URL。
///
/// 顺序按稳定性排：响应头由服务端直接写入，改版概率最低；HTML 正则依赖前端
/// 实现细节；URL 查询串只在重定向场景出现。任一命中即可继续发稿，全部落空
/// 才判定为登录态失效。
fn resolve_auth_token(
    html: &str,
    headers: &reqwest::header::HeaderMap,
    final_url: &str,
) -> Option<String> {
    if let Some(token) = response::header_value(headers, AUTH_TOKEN_HEADERS) {
        return Some(token);
    }
    if let Some(token) = extract_auth_token(html) {
        return Some(token);
    }
    token_from_url(final_url)
}

fn parse_jsonp(body: &str) -> Option<Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).ok();
    }

    let start = trimmed.find('(')?;
    let end = trimmed.rfind(')')?;
    if end <= start {
        return None;
    }

    serde_json::from_str(trimmed[start + 1..end].trim()).ok()
}

fn is_success(value: &Value) -> bool {
    let errno_ok = extract::read_numeric_field(value, &["errno"]) == Some(0);
    let message_ok = value.get("errmsg").and_then(Value::as_str) == Some("success");
    errno_ok || message_ok
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.publish_status && !request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "百家号的正式发布依赖编辑页内的发布按钮（并可能触发人机验证），当前仅支持保存草稿"
                .to_string(),
        ));
    }
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "百家号没有草稿更新接口，已发布文章的同步更新暂不支持，请在后台手动修改".to_string(),
        ));
    }
    Ok(())
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    if let Err((class, message)) = check_operation(request) {
        return PublishResult::failure(class, message, operation, false);
    }

    let cookie = match read_cookie(app) {
        Ok(cookie) => cookie,
        Err((class, message)) => return PublishResult::failure(class, message, operation, false),
    };

    publish_with_cookie(&cookie, request, operation, BASE_URL).await
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    base: &str,
) -> PublishResult {
    let client = response::new_client();
    let editor_url = format!("{}/builder/rc/edit", base);

    let (status, body, headers, final_url) = match response::read_text_with_headers(
        client
            .get(&editor_url)
            .header("Cookie", cookie)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Referer", format!("{}/builder/rc/home", base))
            .header("User-Agent", response::USER_AGENT)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await
    {
        Ok(response) => (response.status, response.body, response.headers, response.final_url),
        Err(failure) => return failure,
    };

    if !(200..300).contains(&status) {
        return response::http_failure(PLATFORM, status, &body, operation, false);
    }

    let auth_token = match resolve_auth_token(&body, &headers, &final_url) {
        Some(token) => token,
        None => {
            return response::auth_failure(
                PLATFORM,
                "未能从编辑页提取发布凭证，请重新登录百家号",
                operation,
                false,
            )
        }
    };

    let content = super::shared::markdown::to_html_content(&request.content);
    // 更新时带上原 article_id，pcui/article/save 才会覆盖旧草稿而不是新建。
    let existing_article_id = if operation == PublishOperation::Update {
        request
            .existing_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty() && !extract::is_placeholder_id(id))
    } else {
        None
    };

    let mut form: Vec<(&str, String)> = vec![
        ("type", "news".to_string()),
        ("title", request.title.clone()),
        ("content", content.clone()),
        ("feed_cat", "1".to_string()),
        ("len", content.chars().count().to_string()),
        (
            "activity_list",
            r#"[{"id":408,"is_checked":0}]"#.to_string(),
        ),
        ("source_reprinted_allow", "0".to_string()),
        ("abstract_from", "1".to_string()),
        ("isBeautify", "false".to_string()),
        ("usingImgFilter", "false".to_string()),
        ("first_exclusive_publish_v2", "3".to_string()),
        ("subtitle", String::new()),
        ("bjhtopic_id", String::new()),
        ("bjhtopic_info", String::new()),
    ];

    if let Some(article_id) = existing_article_id {
        form.push(("article_id", article_id.to_string()));
    }

    let save_url = format!("{}/pcui/article/save?callback=bjhdraft", base);
    let (status, body) = match response::read_text(
        client
            .post(&save_url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("token", &auth_token)
            .header("Origin", base)
            .header("Referer", &editor_url)
            .header("User-Agent", response::USER_AGENT)
            .form(&form)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await
    {
        Ok(pair) => pair,
        Err(failure) => return failure,
    };

    if !(200..300).contains(&status) {
        return response::http_failure(PLATFORM, status, &body, operation, false);
    }

    let value = match parse_jsonp(&body) {
        Some(value) => value,
        None => {
            return response::business_failure(
                PLATFORM,
                "创建草稿失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    if !is_success(&value) {
        let detail = value
            .get("errmsg")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or("创建百家号草稿失败");
        return response::business_failure(PLATFORM, detail, operation, false);
    }

    let article_id = match value
        .get("ret")
        .and_then(|ret| extract::extract_id(ret, &["article_id"]))
    {
        Some(id) => id,
        None => {
            // 覆盖已有草稿时响应可能只回 errno:0 而不带 article_id，复用提交的旧 ID。
            match existing_article_id {
                Some(id) => id.to_string(),
                None => {
                    return response::business_failure(
                        PLATFORM,
                        "创建草稿失败: 响应缺少草稿 ID",
                        operation,
                        false,
                    )
                }
            }
        }
    };

    let edit_url = format!(
        "{}/builder/rc/edit?type=news&article_id={}",
        base, article_id
    );
    let message = format!("百家号草稿已保存！草稿 ID: {}", article_id);

    PublishResult::success(article_id, edit_url, message, operation, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    const EDITOR_HTML: &str =
        r#"<html><script>window.__BJH__INIT__AUTH__ = 'token-abc';</script></html>"#;

    fn sample_request() -> PublishRequest {
        PublishRequest {
            title: "百家号标题".to_string(),
            content: "第一段".to_string(),
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "BDUSS=abc",
            request,
            operation,
            server_base,
        ))
    }

    #[test]
    fn success_returns_draft_id() {
        let server = MockServer::start(
            vec![
                ("GET /builder/rc/edit", 200, EDITOR_HTML),
                (
                    "POST /pcui/article/save",
                    200,
                    r#"bjhdraft({"errno":0,"errmsg":"success","ret":{"article_id":"6600123"}})"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "6600123");
        assert!(!result.is_published);
    }

    #[test]
    fn update_reuses_existing_article_id_when_response_omits_it() {
        let server = MockServer::start(
            vec![
                ("GET /builder/rc/edit", 200, EDITOR_HTML),
                ("POST /pcui/article/save", 200, r#"bjhdraft({"errno":0,"errmsg":"success"})"#),
            ],
            (500, "{}"),
        );

        let mut request = sample_request();
        request.existing_id = Some("6600123".to_string());
        let result = run(&server.base, &request, PublishOperation::Update);

        // 覆盖旧草稿时响应不带 article_id，应复用 6600123 而不是报缺少 ID。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "6600123");
    }

    #[test]
    fn publish_and_published_update_are_not_implemented() {
        let mut request = sample_request();
        request.publish_status = true;
        let err = check_operation(&request).unwrap_err();
        assert_eq!(err.0, FailureClass::NotImplemented);

        let mut request = sample_request();
        request.existing_published = true;
        let err = check_operation(&request).unwrap_err();
        assert_eq!(err.0, FailureClass::NotImplemented);
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(vec![], (401, "{}"));

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start(vec![], (500, r#"{"errmsg":"服务异常"}"#));

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
    }

    #[test]
    fn network_error_maps_to_network_error() {
        let result = run(
            &closed_port_base(),
            &sample_request(),
            PublishOperation::Create,
        );

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NetworkError));
    }

    #[test]
    fn parses_jsonp_wrapper_and_plain_json() {
        let wrapped = parse_jsonp(r#"cb({"errno":0})"#).unwrap();
        assert_eq!(wrapped.get("errno").and_then(Value::as_i64), Some(0));
        let plain = parse_jsonp(r#"{"errno":0}"#).unwrap();
        assert_eq!(plain.get("errno").and_then(Value::as_i64), Some(0));
        assert!(parse_jsonp("not json").is_none());
    }

    #[test]
    fn extracts_auth_token_from_editor_html() {
        assert_eq!(extract_auth_token(EDITOR_HTML), Some("token-abc".to_string()));
        assert_eq!(extract_auth_token("<html></html>"), None);
    }

    #[test]
    fn extracts_auth_token_from_alternate_html_shapes() {
        let object_form = r#"<script>window.__BJH__INIT__AUTH__ = {"token":"tok-obj","uid":1};</script>"#;
        assert_eq!(extract_auth_token(object_form), Some("tok-obj".to_string()));

        let inline_json = r#"<script>var cfg = {"token":"tok-json","uid":1};</script>"#;
        assert_eq!(extract_auth_token(inline_json), Some("tok-json".to_string()));

        let meta_form = r#"<meta name="token" content="tok-meta" />"#;
        assert_eq!(extract_auth_token(meta_form), Some("tok-meta".to_string()));
    }

    #[test]
    fn extracts_token_from_redirect_url() {
        assert_eq!(
            token_from_url("https://baijiahao.baidu.com/builder/rc/edit?type=news&token=tok-url"),
            Some("tok-url".to_string())
        );
        assert_eq!(
            token_from_url("https://baijiahao.baidu.com/builder/rc/edit?type=news"),
            None
        );
        assert_eq!(token_from_url("https://baijiahao.baidu.com/builder/rc/edit"), None);
    }

    #[test]
    fn header_token_wins_over_html_and_url() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("token", "tok-header".parse().unwrap());
        let token = resolve_auth_token(EDITOR_HTML, &headers, "https://x/edit?token=tok-url");
        assert_eq!(token, Some("tok-header".to_string()));
    }

    #[test]
    fn falls_back_to_html_then_url_when_header_absent() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(
            resolve_auth_token(EDITOR_HTML, &headers, "https://x/edit"),
            Some("token-abc".to_string())
        );
        assert_eq!(
            resolve_auth_token("<html></html>", &headers, "https://x/edit?token=tok-url"),
            Some("tok-url".to_string())
        );
        assert_eq!(resolve_auth_token("<html></html>", &headers, "https://x/edit"), None);
    }
}