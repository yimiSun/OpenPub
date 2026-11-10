use serde_json::Value;

use super::shared::{cookie, extract, markdown, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "bilibili_config";
const PLATFORM: &str = "哔哩哔哩";
const API_BASE: &str = "https://api.bilibili.com";
const MEMBER_BASE: &str = "https://member.bilibili.com";
const NAV_PATH: &str = "/x/web-interface/nav?build=0&mobi_app=web";
const DRAFT_PATH: &str = "/x/article/creative/draft/addupdate";
const CSRF_COOKIE: &str = "bili_jct";
const TID_TECH: &str = "4";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.publish_status && !request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "哔哩哔哩的正式发布只能在专栏编辑页内点发布按钮（依赖页面内 XMLHttpRequest 拦截），当前仅支持保存草稿"
                .to_string(),
        ));
    }
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "哔哩哔哩已发布专栏的同步更新暂不支持，请在后台手动修改".to_string(),
        ));
    }
    Ok(())
}

fn error_message(value: &Value) -> String {
    for key in ["message", "msg"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() && trimmed != "0" {
                return trimmed.to_string();
            }
        }
    }
    "登录态已失效，请重新登录哔哩哔哩".to_string()
}

fn parse_nav(body: &str) -> Result<String, (FailureClass, String)> {
    let value = response::parse_json(body).ok_or_else(|| {
        (
            FailureClass::PlatformError,
            format!("{}: 登录状态响应不是有效 JSON: {}", PLATFORM, response::snippet(body)),
        )
    })?;

    let code = extract::read_numeric_field(&value, &["code"]);
    if code != Some(0) {
        return Err((
            FailureClass::AuthInvalid,
            format!("{} {}", PLATFORM, error_message(&value)),
        ));
    }

    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let is_login = data
        .get("isLogin")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !is_login {
        return Err((
            FailureClass::AuthInvalid,
            format!("{} 未登录，请重新登录后重试", PLATFORM),
        ));
    }

    extract::extract_id(&data, &["mid"]).ok_or_else(|| {
        (
            FailureClass::AuthInvalid,
            format!("{} 登录状态响应缺少 mid，请重新登录后重试", PLATFORM),
        )
    })
}

fn build_fields(
    request: &PublishRequest,
    content: &str,
    csrf: &str,
    operation: PublishOperation,
) -> Vec<(String, String)> {
    // 只有更新才复用旧 pgc_id；新建时误用会让接口去改别人的草稿。
    let pgc_id = if operation == PublishOperation::Update {
        request
            .existing_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !extract::is_placeholder_id(id))
            .unwrap_or("0")
    } else {
        "0"
    }
    .to_string();

    vec![
        ("tid".to_string(), TID_TECH.to_string()),
        ("title".to_string(), request.title.clone()),
        ("content".to_string(), content.to_string()),
        ("csrf".to_string(), csrf.to_string()),
        ("save".to_string(), "0".to_string()),
        ("pgc_id".to_string(), pgc_id),
    ]
}

fn draft_edit_url(aid: &str) -> String {
    format!("{}/platform/upload/text/edit?aid={}", MEMBER_BASE, aid)
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

    publish_with_cookie(&cookie, request, operation, API_BASE).await
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    api_base: &str,
) -> PublishResult {
    let csrf = match cookie::require_value(cookie, CSRF_COOKIE, PLATFORM) {
        Ok(value) => value,
        Err(message) => {
            return PublishResult::failure(FailureClass::NotConfigured, message, operation, false)
        }
    };

    let client = response::new_client();

    let nav_url = format!("{}{}", api_base, NAV_PATH);
    let (status, body) = match response::read_text(
        client
            .get(&nav_url)
            .header("Cookie", cookie)
            .header("Accept", "application/json")
            .header("Origin", MEMBER_BASE)
            .header("Referer", format!("{}/", MEMBER_BASE))
            .header("User-Agent", response::USER_AGENT)
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

    if let Err((class, message)) = parse_nav(&body) {
        return PublishResult::failure(class, message, operation, false);
    }

    let content = markdown::to_html_content(&request.content);
    let fields = build_fields(request, &content, &csrf, operation);
    let draft_url = format!("{}{}", api_base, DRAFT_PATH);

    let (status, body) = match response::read_text(
        client
            .post(&draft_url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Origin", MEMBER_BASE)
            .header("Referer", format!("{}/", MEMBER_BASE))
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

    if extract::read_numeric_field(&value, &["code"]) != Some(0) {
        return response::business_failure(PLATFORM, error_message(&value), operation, false);
    }

    let aid = match value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["aid", "id"]))
    {
        Some(id) => id,
        None => {
            // 覆盖已有草稿时接口可能只回 code:0 而不带 aid，复用提交的旧 pgc_id。
            let fallback = if operation == PublishOperation::Update {
                request
                    .existing_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty() && !extract::is_placeholder_id(id))
            } else {
                None
            };
            match fallback {
                Some(id) => id.to_string(),
                None => {
                    return response::business_failure(
                        PLATFORM,
                        "草稿保存失败: 响应缺少专栏 ID",
                        operation,
                        false,
                    )
                }
            }
        }
    };

    let edit_url = draft_edit_url(&aid);
    let message = format!("哔哩哔哩草稿已保存！草稿 ID: {}", aid);

    PublishResult::success(aid, edit_url, message, operation, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    const NAV_OK: &str = r#"{"code":0,"message":"0","data":{"isLogin":true,"mid":10086,"uname":"测试UP主"}}"#;

    fn sample_request() -> PublishRequest {
        PublishRequest {
            title: "B站专栏标题".to_string(),
            content: "第一段正文".to_string(),
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "SESSDATA=abc; bili_jct=csrf-token",
            request,
            operation,
            server_base,
        ))
    }

    #[test]
    fn success_returns_draft_id() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":0,"message":"0","data":{"aid":9527001}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9527001");
        assert!(!result.is_published);
        assert!(result.edit_url.contains("aid=9527001"));
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(vec![], (401, "{}"));

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn nav_not_logged_in_maps_to_auth_invalid() {
        let server = MockServer::start(
            vec![(
                "GET /x/web-interface/nav",
                200,
                r#"{"code":-101,"message":"账号未登录","data":{"isLogin":false}}"#,
            )],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
        assert!(result.message.contains("账号未登录"), "{}", result.message);
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":-400,"message":"csrf 校验失败"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("csrf"), "{}", result.message);
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
    fn missing_csrf_cookie_is_not_configured() {
        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "SESSDATA=abc",
            &sample_request(),
            PublishOperation::Create,
            &closed_port_base(),
        ));

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NotConfigured));
        assert!(result.message.contains("bili_jct"), "{}", result.message);
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
    fn draft_fields_follow_captured_contract() {
        let fields = build_fields(
            &sample_request(),
            "<p>第一段正文</p>",
            "csrf-token",
            PublishOperation::Create,
        );

        let value = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };

        assert_eq!(value("tid"), "4");
        assert_eq!(value("title"), "B站专栏标题");
        assert_eq!(value("csrf"), "csrf-token");
        assert_eq!(value("save"), "0");
        assert_eq!(value("pgc_id"), "0");
    }

    #[test]
    fn update_uses_existing_id_as_pgc_id() {
        let request = PublishRequest {
            existing_id: Some("9527001".to_string()),
            ..sample_request()
        };
        let fields = build_fields(&request, "<p>x</p>", "csrf-token", PublishOperation::Update);
        let pgc_id = fields
            .iter()
            .find(|(key, _)| key == "pgc_id")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(pgc_id, "9527001");
    }

    #[test]
    fn parses_nav_and_rejects_logged_out_payload() {
        assert_eq!(parse_nav(NAV_OK), Ok("10086".to_string()));

        let logged_out = parse_nav(r#"{"code":0,"data":{"isLogin":false}}"#).unwrap_err();
        assert_eq!(logged_out.0, FailureClass::AuthInvalid);
    }
}