use regex::Regex;
use serde_json::Value;

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "51cto_config";
const PLATFORM: &str = "51CTO";
const BASE_URL: &str = "https://blog.51cto.com";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn extract_csrf(html: &str) -> Option<String> {
    let patterns = [
        r#"<meta[^>]*name=["']csrf-token["'][^>]*content=["']([^"']+)["']"#,
        r#"<meta[^>]*content=["']([^"']+)["'][^>]*name=["']csrf-token["']"#,
    ];

    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        if let Some(captures) = regex.captures(html) {
            if let Some(value) = captures.get(1) {
                let token = value.as_str().trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Category {
    parent_id: String,
    child_id: String,
}

fn pick_category(value: &Value) -> Option<Category> {
    for parent in value.as_array()? {
        let parent_id = match extract::extract_id(parent, &["id"]) {
            Some(id) => id,
            None => continue,
        };

        if let Some(children) = parent.get("item").and_then(Value::as_array) {
            for child in children {
                if let Some(child_id) = extract::extract_id(child, &["id"]) {
                    return Some(Category {
                        parent_id,
                        child_id,
                    });
                }
            }
        }

        return Some(Category {
            child_id: parent_id.clone(),
            parent_id,
        });
    }

    None
}

fn is_positive_integer(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) && !value.starts_with('0')
}

fn article_form(
    request: &PublishRequest,
    category: &Category,
    csrf_token: &str,
    draft_id: &str,
    publishing: bool,
) -> Vec<(&'static str, String)> {
    vec![
        ("title", request.title.clone()),
        ("content", request.content.clone()),
        ("pid", category.parent_id.clone()),
        ("cate_id", category.child_id.clone()),
        ("custom_id", "0".to_string()),
        ("tag", request.tags.clone().unwrap_or_default().replace(',', " ")),
        ("abstract", String::new()),
        ("banner_type", "0".to_string()),
        ("blog_type", "1".to_string()),
        ("copy_code", "1".to_string()),
        ("is_hide", "0".to_string()),
        ("top_time", if publishing { String::new() } else { "0".to_string() }),
        ("is_comment", "0".to_string()),
        ("is_old", "0".to_string()),
        ("blog_id", String::new()),
        ("did", draft_id.to_string()),
        ("work_id", String::new()),
        ("class_id", String::new()),
        ("subjectId", String::new()),
        (
            "import_type",
            if publishing { String::new() } else { "-1".to_string() },
        ),
        ("invite_code", String::new()),
        ("raffle", String::new()),
        ("orig", String::new()),
        ("_csrf", csrf_token.to_string()),
        ("check", if publishing { "1".to_string() } else { String::new() }),
    ]
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

    publish_with_cookie(&cookie, request, operation, BASE_URL).await
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    base: &str,
) -> PublishResult {
    let client = response::new_client();

    let editor_url = format!("{}/blogger/publish", base);
    let (status, body) = match response::read_text(
        client
            .get(&editor_url)
            .header("Cookie", cookie)
            .header("User-Agent", response::USER_AGENT)
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Referer", &editor_url)
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

    let csrf_token = match extract_csrf(&body) {
        Some(token) => token,
        None => {
            return response::auth_failure(
                PLATFORM,
                "未能从创作页提取 CSRF Token，请重新登录 51CTO",
                operation,
                false,
            )
        }
    };

    let categories_url = format!("{}/category/get-child", base);
    let (status, body) = match response::read_text(
        client
            .get(&categories_url)
            .header("Cookie", cookie)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("User-Agent", response::USER_AGENT)
            .header("Referer", &editor_url)
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
                "获取分类失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    let business_code = extract::read_numeric_field(&value, &["status", "code"]);
    if business_code != Some(1) {
        let message = value
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("获取分类失败");
        return response::business_failure(PLATFORM, message, operation, false);
    }

    let category = match value.get("data").and_then(pick_category) {
        Some(category) => category,
        None => {
            return response::business_failure(
                PLATFORM,
                "51CTO 未返回可用分类，请先在后台创建博客分类",
                operation,
                false,
            )
        }
    };

    let draft_url = format!("{}/blogger/draft", base);
    // 更新时必须把旧草稿 ID 放进 `did`，否则 51CTO 会把每次同步都当成新建，
    // 表现为草稿箱里不断堆积新草稿。
    let existing_did = if operation == PublishOperation::Update {
        request
            .existing_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty() && !extract::is_placeholder_id(id))
            .unwrap_or_default()
    } else {
        ""
    };
    let form = article_form(request, &category, &csrf_token, existing_did, false);
    let (status, body) = match response::read_text(
        client
            .post(&draft_url)
            .header("Cookie", cookie)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("User-Agent", response::USER_AGENT)
            .header("Referer", &editor_url)
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

    if extract::read_numeric_field(&value, &["status"]) != Some(1) {
        let message = value
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("创建 51CTO 草稿失败");
        return response::business_failure(PLATFORM, message, operation, false);
    }

    let draft_id = match value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["did"]))
    {
        Some(id) => id,
        None => {
            // 更新草稿时响应可能只回 `status:1` 而不带 did，此时复用刚提交的旧 ID，
            // 避免误报「响应缺少草稿 ID」而让用户以为保存失败。
            if !existing_did.is_empty() {
                existing_did.to_string()
            } else {
                return response::business_failure(
                    PLATFORM,
                    "创建草稿失败: 响应缺少草稿 ID",
                    operation,
                    false,
                );
            }
        }
    };

    let mut is_published = request.existing_published;
    let mut post_id = draft_id.clone();

    if request.publish_status && !request.existing_published {
        let publish_url = format!("{}/blogger/publish", base);
        let form = article_form(request, &category, &csrf_token, &draft_id, true);
        let (status, body) = match response::read_text(
            client
                .post(&publish_url)
                .header("Cookie", cookie)
                .header("Accept", "application/json, text/javascript, */*; q=0.01")
                .header("X-Requested-With", "XMLHttpRequest")
                .header("User-Agent", response::USER_AGENT)
                .header("Referer", &editor_url)
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

        if extract::read_numeric_field(&value, &["status"]) != Some(1) {
            let message = value
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("发布 51CTO 文章失败");
            return response::business_failure(PLATFORM, message, operation, false);
        }

        let blog_id = value
            .get("data")
            .and_then(|data| extract::extract_id(data, &["blog_id"]))
            .unwrap_or_default();
        if !is_positive_integer(&blog_id) {
            return response::business_failure(
                PLATFORM,
                "发布结果不明：未返回有效文章 ID",
                operation,
                false,
            );
        }

        post_id = blog_id;
        is_published = true;
    }

    let edit_url = format!("{}/blogger/draft/{}", base, draft_id);
    let message = if is_published {
        format!("已成功发布到 51CTO！文章 ID: {}", post_id)
    } else {
        format!("51CTO 草稿已保存！草稿 ID: {}", draft_id)
    };

    PublishResult::success(post_id, edit_url, message, operation, is_published)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    const EDITOR_HTML: &str =
        r#"<html><head><meta name="csrf-token" content="tok123"></head><body></body></html>"#;

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "51CTO 标题".to_string(),
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
        ))
    }

    #[test]
    fn success_returns_post_id_and_publishes() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                ("POST /blogger/draft", 200, r#"{"status":1,"data":{"did":"d55"}}"#),
                (
                    "POST /blogger/publish",
                    200,
                    r#"{"status":1,"data":{"blog_id":"900"}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "900");
        assert!(result.is_published);
        assert!(result.edit_url.contains("/blogger/draft/d55"));
    }

    #[test]
    fn update_sends_existing_did_and_reuses_it_when_response_omits_it() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                ("POST /blogger/draft", 200, r#"{"status":1}"#),
            ],
            (500, "{}"),
        );

        let mut request = sample_request(false);
        request.existing_id = Some("d77".to_string());
        let result = run(&server.base, &request, PublishOperation::Update);

        // 更新响应不带 did 时应复用旧 ID，而不是报「响应缺少草稿 ID」。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "d77");
        assert!(result.edit_url.contains("/blogger/draft/d77"));
    }

    #[test]
    fn draft_form_carries_did_only_for_update() {
        let category = Category {
            parent_id: "10".to_string(),
            child_id: "11".to_string(),
        };
        let request = sample_request(false);

        let update_form = article_form(&request, &category, "csrf", "d77", false);
        let did = update_form
            .iter()
            .find(|(key, _)| *key == "did")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(did, "d77");

        let create_form = article_form(&request, &category, "csrf", "", false);
        let did = create_form
            .iter()
            .find(|(key, _)| *key == "did")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(did, "");
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
        let server = MockServer::start(vec![], (500, r#"{"msg":"服务异常"}"#));

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
    fn parses_csrf_from_meta_in_either_attribute_order() {
        assert_eq!(
            extract_csrf(r#"<meta content="abc" name="csrf-token">"#),
            Some("abc".to_string())
        );
        assert_eq!(extract_csrf("<html></html>"), None);
    }

    #[test]
    fn category_prefers_child_and_falls_back_to_parent() {
        let value: Value = serde_json::from_str(
            r#"[{"id":10,"item":[{"id":11}]},{"id":20,"item":[]}]"#,
        )
        .unwrap();
        assert_eq!(
            pick_category(&value),
            Some(Category {
                parent_id: "10".to_string(),
                child_id: "11".to_string(),
            })
        );

        let value: Value =
            serde_json::from_str(r#"[{"id":20,"item":[]}]"#).unwrap();
        assert_eq!(
            pick_category(&value),
            Some(Category {
                parent_id: "20".to_string(),
                child_id: "20".to_string(),
            })
        );
    }
}