use serde_json::Value;

use super::shared::{cookie, extract, markdown, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "netease_config";
const PLATFORM: &str = "网易号";
const BASE_URL: &str = "https://mp.163.com";
const NAV_PATH: &str = "/wemedia/navinfo.do";
const PUBLISH_PATH: &str = "/wemedia/article/status/api/publishV2.do";
const EDITOR_BASE: &str = "https://mp.163.com/subscribe_v4/index.html#/article-publish";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Account {
    wemedia_id: String,
    real_user_id: String,
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.publish_status && !request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "网易号的正式发布依赖编辑页运行时生成的 X-NR-Trace-929 加密头与 neg.getToken()，纯 Cookie 直发无法产出，当前仅支持保存草稿"
                .to_string(),
        ));
    }
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "网易号已发布文章的同步更新暂不支持，请在后台手动修改".to_string(),
        ));
    }
    Ok(())
}

fn error_message(value: &Value) -> String {
    for key in ["msg", "message", "error"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "登录态已失效，请重新登录网易号".to_string()
}

/// 「未入驻」是账号状态问题，不是登录态失效。
///
/// 平台原样抛出的「用户未入驻」对用户毫无指导意义——重新登录一百次也不会好，
/// 必须换成一句能照做的提示。这里只在文案里判定：网易号各接口的错误码并未
/// 公开，且不同入口（`navinfo.do` / `publishV2.do`）给出的 code 不一致，
/// 靠「入驻」二字识别反而更稳。
fn is_not_onboarded(message: &str) -> bool {
    message.contains("入驻")
}

fn not_onboarded_message() -> String {
    format!("{} 尚未入驻创作者，请前往网易号官网完成入驻流程", PLATFORM)
}

fn parse_account(body: &str) -> Result<Account, (FailureClass, String)> {
    let value = response::parse_json(body).ok_or_else(|| {
        (
            FailureClass::PlatformError,
            format!("{}: 账号信息响应不是有效 JSON: {}", PLATFORM, response::snippet(body)),
        )
    })?;

    let code = extract::read_numeric_field(&value, &["code"]);
    if code != Some(1) {
        let message = error_message(&value);
        // 未入驻走 `NotConfigured`：前端据此提示「去官网入驻」，而不是引导重新登录。
        if is_not_onboarded(&message) {
            return Err((FailureClass::NotConfigured, not_onboarded_message()));
        }
        return Err((FailureClass::AuthInvalid, format!("{} {}", PLATFORM, message)));
    }

    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let wemedia_id = extract::extract_id(&data, &["tid"]).ok_or_else(|| {
        (
            FailureClass::AuthInvalid,
            format!("{} 账号信息缺少 wemediaId，请重新登录后重试", PLATFORM),
        )
    })?;

    let real_user_id = data
        .get("realUserId")
        .map(|raw| match raw {
            Value::String(text) => text.trim().to_string(),
            Value::Number(number) => number.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();

    Ok(Account {
        wemedia_id,
        real_user_id,
    })
}

fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

fn build_url(base: &str, account: &Account) -> String {
    format!(
        "{}{}?_={}&wemediaId={}&realUserId={}",
        base,
        PUBLISH_PATH,
        timestamp_millis(),
        account.wemedia_id,
        account.real_user_id
    )
}

fn build_fields(
    account: &Account,
    request: &PublishRequest,
    content: &str,
    operation: PublishOperation,
) -> Vec<(String, String)> {
    // 只有更新才带旧 articleId；新建时若误用旧 ID，网易号会去改一篇已存在的文章。
    let article_id = if operation == PublishOperation::Update {
        request
            .existing_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !extract::is_placeholder_id(id))
            .unwrap_or("-1")
    } else {
        "-1"
    }
    .to_string();

    vec![
        ("wemediaId".to_string(), account.wemedia_id.clone()),
        ("articleId".to_string(), article_id),
        ("title".to_string(), request.title.clone()),
        ("content".to_string(), content.to_string()),
        ("cover".to_string(), "threeImg".to_string()),
        ("operation".to_string(), "saveDraft".to_string()),
        ("scheduled".to_string(), "0".to_string()),
        ("ursToken".to_string(), String::new()),
        ("onlineState".to_string(), "1".to_string()),
        ("picUrl".to_string(), String::new()),
        ("original".to_string(), "0".to_string()),
        ("subjectId".to_string(), String::new()),
    ]
}

fn parse_draft_id(value: &Value) -> Option<String> {
    if let Some(data) = value.get("data") {
        if let Some(text) = data.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let candidate = if trimmed.contains('?') || trimmed.contains('=') {
                    draft_id_from_query(trimmed).unwrap_or_default()
                } else {
                    trimmed.to_string()
                };
                if !candidate.is_empty() && !extract::is_placeholder_id(&candidate) {
                    return Some(candidate);
                }
            }
        }
        if let Some(id) = extract::extract_id(data, &["docId", "articleId", "id"]) {
            return Some(id);
        }
    }

    None
}

fn draft_id_from_query(raw: &str) -> Option<String> {
    let query = raw.split_once('?').map(|(_, rest)| rest).unwrap_or(raw);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=')?;
        if name.trim() == "docId" {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
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

async fn fetch_account(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    operation: PublishOperation,
) -> Result<Account, PublishResult> {
    let url = format!("{}{}?_={}", base, NAV_PATH, timestamp_millis());

    let (status, body) = response::read_text(
        client
            .get(&url)
            .header("Cookie", cookie)
            .header("Accept", "application/json")
            .header("Referer", "https://mp.163.com/subscribe_v4/index.html")
            .header("User-Agent", response::USER_AGENT)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await?;

    if !(200..300).contains(&status) {
        return Err(response::http_failure(PLATFORM, status, &body, operation, false));
    }

    parse_account(&body).map_err(|(class, message)| {
        PublishResult::failure(class, message, operation, false)
    })
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    base: &str,
) -> PublishResult {
    let client = response::new_client();

    let account = match fetch_account(&client, base, cookie, operation).await {
        Ok(account) => account,
        Err(failure) => return failure,
    };

    let content = markdown::to_html_content(&request.content);
    let fields = build_fields(&account, request, &content, operation);
    let url = build_url(base, &account);

    let (status, body) = match response::read_text(
        client
            .post(&url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Origin", "https://mp.163.com")
            .header("Referer", "https://mp.163.com/subscribe_v4/index.html")
            .header("User-Agent", response::USER_AGENT)
            .form(&fields)
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

    let value = match response::parse_json(&body) {
        Some(value) => value,
        None => {
            return response::business_failure(
                PLATFORM,
                format!("草稿保存失败: 响应不是有效 JSON: {}", response::snippet(&body)),
                operation,
                false,
            )
        }
    };

    if extract::read_numeric_field(&value, &["code"]) != Some(1) {
        let message = error_message(&value);
        if is_not_onboarded(&message) {
            return PublishResult::failure(
                FailureClass::NotConfigured,
                not_onboarded_message(),
                operation,
                false,
            );
        }
        return response::business_failure(PLATFORM, message, operation, false);
    }

    let draft_id = match parse_draft_id(&value) {
        Some(id) => id,
        None => {
            // 更新时接口可能只回成功标志而不带草稿 ID，复用已提交的旧 ID。
            let fallback = if operation == PublishOperation::Update {
                request
                    .existing_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty() && !extract::is_placeholder_id(id))
                    .map(str::to_string)
            } else {
                None
            };

            match fallback {
                Some(id) => id,
                None => {
                    return response::business_failure(
                        PLATFORM,
                        "草稿保存失败: 响应缺少草稿 ID",
                        operation,
                        false,
                    )
                }
            }
        }
    };

    let edit_url = format!("{}/{}?option=editDraft", EDITOR_BASE, draft_id);
    let message = format!("网易号草稿已保存！草稿 ID: {}", draft_id);

    PublishResult::success(draft_id, edit_url, message, operation, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    const NAV_OK: &str = r#"{"code":1,"msg":"ok","data":{"tid":123456,"tname":"测试账号","realUserId":8899}}"#;

    fn sample_request() -> PublishRequest {
        PublishRequest {
            title: "网易号标题".to_string(),
            content: "第一段正文".to_string(),
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "NTES_SESS=abc",
            request,
            operation,
            server_base,
        ))
    }

    #[test]
    fn success_returns_draft_id() {
        let server = MockServer::start(
            vec![
                ("GET /wemedia/navinfo.do", 200, NAV_OK),
                (
                    "POST /wemedia/article/status/api/publishV2.do",
                    200,
                    r#"{"code":1,"msg":"ok","data":"https://mp.163.com/article/draft?docId=9f2c1a"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9f2c1a");
        assert!(!result.is_published);
        assert!(result.edit_url.contains("option=editDraft"));
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(vec![], (401, "{}"));

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn nav_code_mismatch_maps_to_auth_invalid() {
        let server = MockServer::start(
            vec![("GET /wemedia/navinfo.do", 200, r#"{"code":0,"msg":"未登录"}"#)],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
        assert!(result.message.contains("未登录"), "{}", result.message);
    }

    #[test]
    fn not_onboarded_account_gets_actionable_hint() {
        let server = MockServer::start(
            vec![(
                "GET /wemedia/navinfo.do",
                200,
                r#"{"code":0,"msg":"用户未入驻"}"#,
            )],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NotConfigured));
        assert!(result.message.contains("入驻流程"), "{}", result.message);
        assert!(
            !result.message.contains("登录态已失效"),
            "未入驻不该被当成登录失效: {}",
            result.message
        );
    }

    #[test]
    fn not_onboarded_is_detected_by_wording() {
        assert!(is_not_onboarded("用户未入驻"));
        assert!(is_not_onboarded("该账号尚未入驻网易号"));
        assert!(!is_not_onboarded("未登录"));
        assert!(!is_not_onboarded("标题包含敏感词"));
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start(
            vec![
                ("GET /wemedia/navinfo.do", 200, NAV_OK),
                (
                    "POST /wemedia/article/status/api/publishV2.do",
                    200,
                    r#"{"code":0,"msg":"标题包含敏感词"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("敏感词"), "{}", result.message);
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
    fn publish_requires_editor_page_and_is_not_implemented() {
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
    fn parses_account_and_rejects_missing_tid() {
        let account = parse_account(NAV_OK).unwrap();
        assert_eq!(account.wemedia_id, "123456");
        assert_eq!(account.real_user_id, "8899");

        let missing = parse_account(r#"{"code":1,"data":{}}"#).unwrap_err();
        assert_eq!(missing.0, FailureClass::AuthInvalid);
    }

    #[test]
    fn draft_fields_follow_captured_contract() {
        let account = Account {
            wemedia_id: "123456".to_string(),
            real_user_id: "8899".to_string(),
        };
        let fields = build_fields(
            &account,
            &sample_request(),
            "<p>第一段正文</p>",
            PublishOperation::Create,
        );

        let value = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };

        assert_eq!(value("wemediaId"), "123456");
        assert_eq!(value("articleId"), "-1");
        assert_eq!(value("cover"), "threeImg");
        assert_eq!(value("operation"), "saveDraft");
        assert_eq!(value("scheduled"), "0");
        assert_eq!(value("onlineState"), "1");
        assert_eq!(value("original"), "0");
        assert_eq!(value("ursToken"), "");
    }

    #[test]
    fn update_field_carries_existing_article_id() {
        let account = Account {
            wemedia_id: "123456".to_string(),
            real_user_id: "8899".to_string(),
        };
        let mut request = sample_request();
        request.existing_id = Some("5500".to_string());

        let fields = build_fields(&account, &request, "<p>x</p>", PublishOperation::Update);
        let article_id = fields
            .iter()
            .find(|(key, _)| key == "articleId")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(article_id, "5500");
    }

    #[test]
    fn url_carries_wemedia_and_real_user_id() {
        let account = Account {
            wemedia_id: "123456".to_string(),
            real_user_id: "8899".to_string(),
        };
        let url = build_url("https://mp.163.com", &account);
        assert!(url.contains("/wemedia/article/status/api/publishV2.do?"), "{}", url);
        assert!(url.contains("wemediaId=123456"), "{}", url);
        assert!(url.contains("realUserId=8899"), "{}", url);
    }

    #[test]
    fn parses_draft_id_from_query_and_plain_text() {
        let wrapped = serde_json::json!({
            "code": 1,
            "data": "https://mp.163.com/article/draft?docId=abc123&from=mp"
        });
        assert_eq!(parse_draft_id(&wrapped), Some("abc123".to_string()));

        let plain = serde_json::json!({ "code": 1, "data": "abc123" });
        assert_eq!(parse_draft_id(&plain), Some("abc123".to_string()));

        let missing = serde_json::json!({ "code": 1, "data": "" });
        assert_eq!(parse_draft_id(&missing), None);
    }
}