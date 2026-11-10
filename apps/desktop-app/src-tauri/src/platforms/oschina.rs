use serde_json::{json, Value};

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "oschina_config";
const PLATFORM: &str = "开源中国";
const API_BASE: &str = "https://apiv1.oschina.net";
const HOME_BASE: &str = "https://my.oschina.net";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn business_message(value: &Value) -> Option<String> {
    for key in ["message", "msg", "error"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn is_business_success(value: &Value) -> bool {
    if value.get("error").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    value.get("success").and_then(Value::as_bool) == Some(true)
}

fn positive_id(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let parsed = trimmed.parse::<i64>().ok()?;
    if parsed > 0 {
        Some(parsed)
    } else {
        None
    }
}

/// `result` 既可能是 `{"id": 123}`，也可能是裸的 `123`（发布接口返回标量）。
fn result_id(value: &Value) -> Option<String> {
    let result = value.get("result")?;
    match result {
        Value::Object(_) => extract::extract_id(result, &["id"]),
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let cookie = match read_cookie(app) {
        Ok(cookie) => cookie,
        Err((class, message)) => return PublishResult::failure(class, message, operation, false),
    };

    publish_with_cookie(&cookie, request, operation, API_BASE, HOME_BASE).await
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    api_base: &str,
    home_base: &str,
) -> PublishResult {
    let client = response::new_client();

    let details_url = format!("{}/oschinapi/user/myDetails", api_base);
    let (status, body) = match response::read_text(
        client
            .get(&details_url)
            .header("Cookie", cookie)
            .header("Accept", "application/json")
            .header("Origin", home_base)
            .header("Referer", format!("{}/", home_base))
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

    let value = match response::parse_json(&body) {
        Some(value) => value,
        None => {
            return response::business_failure(
                PLATFORM,
                "获取用户信息失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    let user_id = match value
        .get("result")
        .and_then(|result| extract::extract_id(result, &["userId"]))
        .and_then(|id| positive_id(&id))
    {
        Some(id) => id,
        None => {
            let detail = business_message(&value)
                .unwrap_or_else(|| "未登录，请先登录开源中国".to_string());
            return response::auth_failure(PLATFORM, &detail, operation, false);
        }
    };

    let content_type = if request.content.trim().is_empty() { 2 } else { 1 };
    // 更新时必须带上原草稿 ID，否则 save_draft 会新建一份草稿而不是覆盖。
    let existing_draft = if operation == PublishOperation::Update {
        request.existing_id.as_deref().and_then(positive_id)
    } else {
        None
    };

    let mut draft_payload = json!({
        "title": request.title,
        "user": user_id,
        "content": request.content,
        "contentType": content_type,
        "catalog": 0,
        "originUrl": "",
        "privacy": true,
        "disableComment": false,
    });

    if let (Some(draft), Some(object)) = (existing_draft, draft_payload.as_object_mut()) {
        object.insert("draft".to_string(), json!(draft));
    }

    let draft_url = format!("{}/oschinapi/api/draft/save_draft", api_base);
    let (status, body) = match response::read_text(
        client
            .post(&draft_url)
            .header("Cookie", cookie)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Origin", home_base)
            .header("Referer", format!("{}/", home_base))
            .header("User-Agent", response::USER_AGENT)
            .json(&draft_payload)
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
                "创建草稿失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    let draft_id = match result_id(&value) {
        Some(id) => id,
        None => {
            // 覆盖已有草稿时接口可能只回成功标志而不带新 ID，复用提交的旧 ID。
            if let Some(draft) = existing_draft {
                draft.to_string()
            } else {
                let detail = business_message(&value)
                    .unwrap_or_else(|| "创建草稿失败: 响应缺少草稿 ID".to_string());
                return response::business_failure(PLATFORM, detail, operation, false);
            }
        }
    };

    let mut is_published = request.existing_published;
    let mut post_id = draft_id.clone();

    if request.publish_status && !request.existing_published {
        let numeric_draft = match positive_id(&draft_id) {
            Some(id) => id,
            None => {
                return response::business_failure(
                    PLATFORM,
                    "草稿 ID 格式异常，无法发布",
                    operation,
                    false,
                )
            }
        };

        let publish_payload = json!({
            "title": request.title,
            "user": user_id,
            "content": request.content,
            "contentType": content_type,
            "type": "1",
            "originUrl": "",
            "catalog": 0,
            "privacy": true,
            "disableComment": false,
            "draft": numeric_draft,
            "isAiRelated": false,
        });

        let publish_url = format!("{}/oschinapi/blog/web/add", api_base);
        let (status, body) = match response::read_text(
            client
                .post(&publish_url)
                .header("Cookie", cookie)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .header("Origin", home_base)
                .header("Referer", format!("{}/", home_base))
                .header("User-Agent", response::USER_AGENT)
                .json(&publish_payload)
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
                    "发布响应不是有效 JSON，无法确认是否发布成功",
                    operation,
                    false,
                )
            }
        };

        if !is_business_success(&value) {
            let detail = business_message(&value)
                .unwrap_or_else(|| "开源中国拒绝发布请求".to_string());
            return response::business_failure(PLATFORM, detail, operation, false);
        }

        let article_id = result_id(&value).unwrap_or_default();
        if positive_id(&article_id).is_none() {
            return response::business_failure(
                PLATFORM,
                "发布结果不明：未返回有效文章 ID",
                operation,
                false,
            );
        }

        post_id = article_id;
        is_published = true;
    }

    let edit_url = format!(
        "{}/u/{}/blog/ai-write/draft/{}",
        home_base, user_id, draft_id
    );
    let message = if is_published {
        format!("已成功发布到开源中国！文章 ID: {}", post_id)
    } else {
        format!("开源中国草稿已保存！草稿 ID: {}", draft_id)
    };

    PublishResult::success(post_id, edit_url, message, operation, is_published)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "开源中国标题".to_string(),
            content: "## 正文".to_string(),
            publish_status,
            ..Default::default()
        }
    }

    fn run(
        server_base: &str,
        request: &PublishRequest,
        operation: PublishOperation,
    ) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "session=abc",
            request,
            operation,
            server_base,
            server_base,
        ))
    }

    #[test]
    fn update_reuses_existing_draft_when_response_omits_id() {
        let server = MockServer::start(
            vec![
                (
                    "GET /oschinapi/user/myDetails",
                    200,
                    r#"{"success":true,"result":{"userId":7788}}"#,
                ),
                (
                    "POST /oschinapi/api/draft/save_draft",
                    200,
                    r#"{"success":true}"#,
                ),
            ],
            (500, "{}"),
        );

        let mut request = sample_request(false);
        request.existing_id = Some("9001".to_string());
        let result = run(&server.base, &request, PublishOperation::Update);

        // 覆盖已有草稿时响应不带 id，应复用 9001 而不是报「响应缺少草稿 ID」。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9001");
        assert!(result.edit_url.contains("/blog/ai-write/draft/9001"));
    }

    #[test]
    fn success_returns_post_id_and_publishes() {
        let server = MockServer::start(
            vec![
                (
                    "GET /oschinapi/user/myDetails",
                    200,
                    r#"{"success":true,"result":{"userId":7788}}"#,
                ),
                (
                    "POST /oschinapi/api/draft/save_draft",
                    200,
                    r#"{"success":true,"result":{"id":9001}}"#,
                ),
                (
                    "POST /oschinapi/blog/web/add",
                    200,
                    r#"{"success":true,"result":55501}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "55501");
        assert!(result.is_published);
        assert!(result.edit_url.contains("/blog/ai-write/draft/9001"));
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(vec![], (401, "{}"));

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start(vec![], (500, r#"{"message":"服务异常"}"#));

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
    }

    #[test]
    fn network_error_maps_to_network_error() {
        let result = run(
            &closed_port_base(),
            &sample_request(false),
            PublishOperation::Create,
        );

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NetworkError));
    }

    #[test]
    fn draft_only_request_stops_before_publish() {
        let server = MockServer::start(
            vec![
                (
                    "GET /oschinapi/user/myDetails",
                    200,
                    r#"{"success":true,"result":{"userId":7788}}"#,
                ),
                (
                    "POST /oschinapi/api/draft/save_draft",
                    200,
                    r#"{"success":true,"result":{"id":9001}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9001");
        assert!(!result.is_published);
    }
}