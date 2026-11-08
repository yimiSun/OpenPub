use super::shared::{cookie, markdown, truncate};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "juejin_config";
const ERROR_SNIPPET_CHARS: usize = 300;
const DRAFT_CREATE_URL: &str = "https://api.juejin.cn/content_api/v1/article_draft/create";
const DRAFT_UPDATE_URL: &str = "https://api.juejin.cn/content_api/v1/article_draft/update";
const ARTICLE_UPDATE_URL: &str = "https://api.juejin.cn/content_api/v1/article/update";
const DEFAULT_CATEGORY_ID: &str = "6809637773935378440";
const DRAFT_EDIT_TYPE: i64 = 10;

fn truncate_chars(input: &str, limit: usize) -> String {
    truncate::truncate_chars(input, limit)
}

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, "掘金")
}

pub fn resolve_url(
    request: &PublishRequest,
    operation: PublishOperation,
) -> Result<&'static str, (FailureClass, String)> {
    if operation == PublishOperation::Create && request.publish_status {
        return Err((
            FailureClass::NotImplemented,
            "掘金目前仅支持保存草稿，请到掘金后台手动发布".to_string(),
        ));
    }

    if operation == PublishOperation::Update && read_target_id(request).is_empty() {
        return Err((
            FailureClass::NotConfigured,
            "更新文章需要已有的文章 ID，但当前映射记录缺少该 ID".to_string(),
        ));
    }

    Ok(match operation {
        PublishOperation::Create => DRAFT_CREATE_URL,
        PublishOperation::Update => {
            if request.existing_published {
                ARTICLE_UPDATE_URL
            } else {
                DRAFT_UPDATE_URL
            }
        }
    })
}

fn read_err_no(response: &serde_json::Value) -> i64 {
    match response.get("err_no") {
        Some(v) => {
            if let Some(n) = v.as_i64() {
                n
            } else if let Some(s) = v.as_str() {
                s.trim().parse::<i64>().unwrap_or(-1)
            } else {
                -1
            }
        }
        None => -1,
    }
}

fn is_placeholder_id(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    trimmed.is_empty() || trimmed == "0" || trimmed == "(未知)" || trimmed == "未知"
}

fn value_to_id(value: &serde_json::Value) -> Option<String> {
    let candidate = if let Some(s) = value.as_str() {
        s.trim().to_string()
    } else if let Some(n) = value.as_i64() {
        n.to_string()
    } else if let Some(n) = value.as_u64() {
        n.to_string()
    } else {
        return None;
    };

    if is_placeholder_id(&candidate) {
        None
    } else {
        Some(candidate)
    }
}

pub fn extract_id(data: &serde_json::Value) -> Option<String> {
    const DIRECT_KEYS: [&str; 5] = ["id", "draft_id", "article_id", "articleId", "post_id"];
    for key in DIRECT_KEYS {
        if let Some(v) = data.get(key) {
            if let Some(id) = value_to_id(v) {
                return Some(id);
            }
        }
    }

    for key in ["draft", "article"] {
        if let Some(nested) = data.get(key) {
            if let Some(id) = extract_id(nested) {
                return Some(id);
            }
        }
    }

    None
}

fn read_target_id(request: &PublishRequest) -> String {
    let raw = request
        .existing_id
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if is_placeholder_id(&raw) {
        String::new()
    } else {
        raw
    }
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let cookie = match read_cookie(app) {
        Ok(c) => c,
        Err((class, message)) => {
            return PublishResult::failure(class, message, operation, false);
        }
    };

    let target_id = read_target_id(request);

    let url = match resolve_url(request, operation) {
        Ok(u) => u,
        Err((class, message)) => {
            return PublishResult::failure(class, message, operation, false);
        }
    };

    let request_body = build_body(request, operation, &target_id);

    let client = reqwest::Client::new();
    let response = match client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Cookie", &cookie)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Referer", "https://juejin.cn/")
        .header("Origin", "https://juejin.cn")
        .json(&request_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return PublishResult::failure(
                FailureClass::NetworkError,
                format!("请求发送失败: {}", e),
                operation,
                false,
            )
        }
    };

    let status = response.status();
    let response_text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            return PublishResult::failure(
                FailureClass::NetworkError,
                format!("读取响应失败: {}", e),
                operation,
                false,
            )
        }
    };

    eprintln!("[juejin] 请求地址: {}，请求体: {}", url, request_body);
    eprintln!("[juejin] 原始响应: {}", response_text);

    let response_json: serde_json::Value =
        serde_json::from_str(&response_text).unwrap_or_else(|_| serde_json::json!({}));

    let err_no = read_err_no(&response_json);

    if err_no == 0 {
        let data = response_json
            .get("data")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let returned_id = match extract_id(&data) {
            Some(id) => id,
            None => {
                return PublishResult::failure(
                    FailureClass::PlatformError,
                    format!(
                        "掘金返回成功但未能提取文章 ID，原始响应: {}",
                        truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
                    ),
                    operation,
                    request.existing_published,
                )
            }
        };

        let edit_url = format!("https://juejin.cn/editor/drafts/{}", returned_id);
        let action = match (operation, request.existing_published) {
            (PublishOperation::Update, true) => "已发布文章更新成功",
            (PublishOperation::Update, false) => "草稿更新成功",
            (PublishOperation::Create, _) => "草稿创建成功",
        };

        let mut message = format!("{}! 文章 ID: {}", action, returned_id);
        if request.publish_status && !request.existing_published {
            message.push_str("（掘金仅支持草稿，本次已保存为草稿，请到掘金后台手动发布）");
        }

        return PublishResult::success(
            returned_id.clone(),
            edit_url,
            message,
            operation,
            request.existing_published,
        );
    }

    let err_msg = response_json
        .get("err_msg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let detail = if err_msg.is_empty() {
        truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
    } else {
        err_msg
    };

    let class = if status.as_u16() == 401 || err_no == 401 {
        FailureClass::AuthInvalid
    } else {
        FailureClass::PlatformError
    };

    PublishResult::failure(
        class,
        format!("掘金 API 错误 (err_no: {}, HTTP {}): {}", err_no, status, detail),
        operation,
        false,
    )
}

fn build_body(request: &PublishRequest, operation: PublishOperation, target_id: &str) -> serde_json::Value {
    let mut body = serde_json::json!({
        "title": request.title,
        "mark_content": request.content,
        "content": markdown::to_html_content(&request.content),
        "category_id": DEFAULT_CATEGORY_ID,
        "tag_ids": Vec::<String>::new(),
        "brief_content": "",
        "edit_type": DRAFT_EDIT_TYPE,
    });

    if operation == PublishOperation::Update {
        if request.existing_published {
            body["article_id"] = serde_json::Value::String(target_id.to_string());
        } else {
            body["draft_id"] = serde_json::Value::String(target_id.to_string());
            body["id"] = serde_json::Value::String(target_id.to_string());
        }
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(publish_status: bool, is_update: bool, existing_id: Option<&str>) -> PublishRequest {
        request_with_state(publish_status, is_update, existing_id, false)
    }

    fn request_with_state(
        publish_status: bool,
        is_update: bool,
        existing_id: Option<&str>,
        existing_published: bool,
    ) -> PublishRequest {
        PublishRequest {
            title: "标题".to_string(),
            content: "# 正文".to_string(),
            existing_id: existing_id.map(|s| s.to_string()),
            is_update,
            publish_status,
            existing_published,
        }
    }

    #[test]
    fn create_live_returns_not_implemented() {
        let result = resolve_url(&request(true, false, None), PublishOperation::Create);
        assert!(result.is_err());
        let (class, message) = result.err().unwrap();
        assert_eq!(class, FailureClass::NotImplemented);
        assert!(message.contains("仅支持保存草稿"));
    }

    #[test]
    fn update_without_existing_id_is_rejected() {
        let result = resolve_url(&request(false, true, None), PublishOperation::Update);
        assert!(result.is_err());
        let (class, _) = result.err().unwrap();
        assert_eq!(class, FailureClass::NotConfigured);
    }

    #[test]
    fn update_uses_draft_update_url() {
        let url = resolve_url(&request(false, true, Some("123")), PublishOperation::Update)
            .expect("should resolve");
        assert_eq!(url, DRAFT_UPDATE_URL);
        assert_ne!(url, DRAFT_CREATE_URL);
    }

    #[test]
    fn unpublished_update_ignores_overwrite_and_stays_on_draft() {
        let url = resolve_url(
            &request_with_state(true, true, Some("123"), false),
            PublishOperation::Update,
        )
        .expect("未发布文章必须走草稿更新分支");
        assert_eq!(url, DRAFT_UPDATE_URL);
        assert_ne!(url, ARTICLE_UPDATE_URL);
    }

    #[test]
    fn unpublished_update_body_carries_draft_id_even_with_overwrite() {
        let body = build_body(
            &request_with_state(true, true, Some("123"), false),
            PublishOperation::Update,
            "123",
        );
        assert_eq!(body.get("draft_id").and_then(|v| v.as_str()), Some("123"));
        assert!(body.get("article_id").is_none());
    }

    #[test]
    fn published_update_uses_article_update_url() {
        let url = resolve_url(
            &request_with_state(true, true, Some("123"), true),
            PublishOperation::Update,
        )
        .expect("should resolve");
        assert_eq!(url, ARTICLE_UPDATE_URL);
        assert_ne!(url, DRAFT_CREATE_URL);
    }

    #[test]
    fn create_draft_uses_draft_create_url() {
        let url = resolve_url(&request(false, false, None), PublishOperation::Create)
            .expect("should resolve");
        assert_eq!(url, DRAFT_CREATE_URL);
    }

    #[test]
    fn update_body_carries_draft_id() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123");
        assert_eq!(body.get("draft_id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn draft_update_body_carries_both_id_and_draft_id() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123");
        assert_eq!(body.get("draft_id").and_then(|v| v.as_str()), Some("123"));
        assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn body_uses_official_category_id_and_edit_type() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123");
        assert_eq!(
            body.get("category_id").and_then(|v| v.as_str()),
            Some(DEFAULT_CATEGORY_ID)
        );
        assert_eq!(
            body.get("edit_type").and_then(|v| v.as_i64()),
            Some(DRAFT_EDIT_TYPE)
        );
        assert_ne!(
            body.get("category_id").and_then(|v| v.as_str()),
            Some("0")
        );
    }

    #[test]
    fn body_carries_mark_content_and_empty_tag_brief() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123");
        assert_eq!(
            body.get("mark_content").and_then(|v| v.as_str()),
            Some("# 正文")
        );
        let tags = body.get("tag_ids").and_then(|v| v.as_array()).expect("tag_ids 必须是数组");
        assert!(tags.is_empty(), "tag_ids 必须为空数组");
        assert_eq!(
            body.get("brief_content").and_then(|v| v.as_str()),
            Some("")
        );
    }

    #[test]
    fn body_content_is_html_while_mark_content_is_markdown() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123");
        let content = body.get("content").and_then(|v| v.as_str()).expect("content 必须存在");
        assert!(content.starts_with("<p>"), "content 应为 HTML 段落，实际: {}", content);
        assert!(content.ends_with("</p>"), "content 应为 HTML 段落，实际: {}", content);
        assert_eq!(
            body.get("mark_content").and_then(|v| v.as_str()),
            Some("# 正文")
        );
    }

    #[test]
    fn html_content_escapes_special_characters() {
        let html = markdown::to_html_content("a < b & c");
        assert_eq!(html, "<p>a &lt; b &amp; c</p>");
    }

    #[test]
    fn published_update_body_carries_article_id() {
        let body = build_body(
            &request_with_state(false, true, Some("123"), true),
            PublishOperation::Update,
            "123",
        );
        assert_eq!(body.get("article_id").and_then(|v| v.as_str()), Some("123"));
        assert!(body.get("draft_id").is_none());
        assert!(body.get("id").is_none());
    }

    #[test]
    fn create_body_has_no_draft_id() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "");
        assert!(body.get("draft_id").is_none());
        assert!(body.get("id").is_none());
        assert!(body.get("article_id").is_none());
    }

    #[test]
    fn extract_id_reads_data_id() {
        let data = serde_json::json!({ "id": "7123456789" });
        assert_eq!(extract_id(&data), Some("7123456789".to_string()));
    }

    #[test]
    fn extract_id_reads_nested_draft_id() {
        let data = serde_json::json!({ "draft_id": "abc123" });
        assert_eq!(extract_id(&data), Some("abc123".to_string()));
    }

    #[test]
    fn extract_id_reads_numeric_id() {
        let data = serde_json::json!({ "id": 12345678 });
        assert_eq!(extract_id(&data), Some("12345678".to_string()));
    }

    #[test]
    fn extract_id_returns_none_when_absent() {
        let data = serde_json::json!({ "foo": "bar" });
        assert_eq!(extract_id(&data), None);
    }

    #[test]
    fn extract_id_skips_placeholder_article_id_and_prefers_draft_id() {
        let data = serde_json::json!({ "draft_id": "7123456789", "article_id": "0" });
        assert_eq!(extract_id(&data), Some("7123456789".to_string()));
    }

    #[test]
    fn extract_id_returns_none_when_only_placeholder_ids() {
        let data = serde_json::json!({ "article_id": "0", "draft_id": 0 });
        assert_eq!(extract_id(&data), None);
    }

    #[test]
    fn update_with_placeholder_existing_id_is_rejected() {
        for id in ["0", "", "未知", "(未知)"] {
            let result = resolve_url(&request(false, true, Some(id)), PublishOperation::Update);
            assert!(result.is_err(), "占位 ID {} 必须被拒绝", id);
            let (class, _) = result.err().unwrap();
            assert_eq!(class, FailureClass::NotConfigured);
        }
    }

    #[test]
    fn read_err_no_accepts_string_and_number() {
        assert_eq!(read_err_no(&serde_json::json!({ "err_no": 0 })), 0);
        assert_eq!(read_err_no(&serde_json::json!({ "err_no": "0" })), 0);
        assert_eq!(read_err_no(&serde_json::json!({ "err_no": 2 })), 2);
        assert_eq!(read_err_no(&serde_json::json!({})), -1);
    }
}