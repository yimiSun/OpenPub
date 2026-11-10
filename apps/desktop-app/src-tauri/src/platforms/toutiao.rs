use serde_json::{json, Value};

use super::shared::{cookie, csrf, extract, markdown, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "toutiao_config";
const PLATFORM: &str = "今日头条";
const BASE_URL: &str = "https://mp.toutiao.com";
const EDITOR_URL: &str = "https://mp.toutiao.com/profile_v4/graphic/publish";
const PUBLISH_PATH: &str = "/mp/agw/article/publish?source=mp&type=article&aid=1231";
const CSRF_PROBE_PATH: &str = "/ttwid/check/";
/// 头条的 CSRF 探针版本号，由头条前端 SDK 固定，必须随探针头一起发送。
const CSRF_VERSION: &str = "1.2.22";
/// 探针取回的是 `x-ware-csrf-token`，发往业务接口时要改名成这个头。
const BUSINESS_CSRF_HEADER: &str = "x-secsdk-csrf-token";

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "今日头条不支持修改已发布的文章，请先在头条后台将其转为草稿后再同步".to_string(),
        ));
    }
    Ok(())
}

fn business_error(value: &Value, fallback: &str) -> String {
    value
        .get("message")
        .or_else(|| value.get("msg"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn article_form(
    request: &PublishRequest,
    html: &str,
    pgc_id: &str,
    publish: bool,
) -> Vec<(&'static str, String)> {
    let mut extra = json!({
        "content_source": 100000000402_i64,
        "content_word_cnt": html.chars().count(),
        "is_multi_title": 0,
        "sub_titles": [],
        "gd_ext": {
            "entrance": "",
            "from_page": "publisher_mp",
            "enter_from": "PC",
            "device_platform": "mp",
            "is_message": 0,
        },
    });
    if publish {
        if let Some(object) = extra.as_object_mut() {
            object.insert("tuwen_wtt_trans_flag".to_string(), json!("2"));
            object.insert("info_source".to_string(), json!({ "source_type": -1 }));
        }
    }

    let title_id = format!(
        "{}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        uuid::Uuid::new_v4().simple().to_string()
            .chars()
            .take(6)
            .collect::<String>()
    );

    let mut form: Vec<(&'static str, String)> = vec![
        ("article_type", "0".to_string()),
        ("pgc_id", pgc_id.to_string()),
        ("source", "29".to_string()),
        ("extra", extra.to_string()),
        ("content", html.to_string()),
        ("title", request.title.clone()),
        (
            "search_creation_info",
            r#"{"searchTopOne":0,"abstract":""}"#.to_string(),
        ),
        ("title_id", title_id),
        ("mp_editor_stat", "{}".to_string()),
        ("is_refute_rumor", "0".to_string()),
        ("save", if publish { "1" } else { "0" }.to_string()),
        ("entrance", if publish { "main" } else { "" }.to_string()),
        ("timer_status", "0".to_string()),
        ("timer_time", String::new()),
        ("educluecard", String::new()),
        ("draft_form_data", r#"{"coverType":3}"#.to_string()),
        ("pgc_feed_covers", "[]".to_string()),
    ];

    if publish {
        form.extend(vec![
            ("ic_uri_list", String::new()),
            ("appid_list", String::new()),
            ("stock_ids", String::new()),
            ("concern_list", String::new()),
        ]);
    }

    form
}

async fn submit_article(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    csrf_token: &str,
    request: &PublishRequest,
    html: &str,
    pgc_id: &str,
    publish: bool,
    operation: PublishOperation,
) -> Result<Value, PublishResult> {
    let url = format!("{}{}", base, PUBLISH_PATH);
    let form = article_form(request, html, pgc_id, publish);

    let (status, body) = response::read_text(
        client
            .post(&url)
            .header("Cookie", cookie)
            .header(BUSINESS_CSRF_HEADER, csrf_token)
            .header("Origin", "https://mp.toutiao.com")
            .header("Referer", EDITOR_URL)
            .header("User-Agent", response::USER_AGENT)
            .form(&form)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await?;

    if !(200..300).contains(&status) {
        return Err(response::http_failure(
            PLATFORM,
            status,
            &body,
            operation,
            false,
        ));
    }

    let value = match response::parse_json(&body) {
        Some(value) => value,
        None => {
            return Err(response::business_failure(
                PLATFORM,
                if publish {
                    "发布响应不是有效 JSON，无法确认是否发布成功"
                } else {
                    "保存草稿失败: 响应不是有效 JSON"
                },
                operation,
                false,
            ))
        }
    };

    if extract::read_numeric_field(&value, &["err_no"]) != Some(0) {
        let detail = business_error(
            &value,
            if publish {
                "今日头条拒绝发布请求"
            } else {
                "保存今日头条草稿失败"
            },
        );
        return Err(response::business_failure(PLATFORM, detail, operation, false));
    }

    Ok(value)
}

fn article_id(value: &Value) -> Option<String> {
    extract::extract_id(value, &["pgc_id"])
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    base: &str,
) -> PublishResult {
    let client = response::new_client();

    let probe_url = format!("{}{}", base, CSRF_PROBE_PATH);
    let csrf_token = match csrf::probe_csrf_token(
        &client,
        &probe_url,
        Some(cookie),
        CSRF_VERSION,
        csrf::TokenSelect::Whole,
    )
    .await
    {
        Ok(token) => token,
        Err(error) => {
            let (class, message) = error.into_platform_error();
            return PublishResult::failure(
                class,
                format!("{}: {}", PLATFORM, message),
                operation,
                false,
            )
        }
    };

    let html = markdown::to_html_content(&request.content);

    let draft_id = match operation {
        PublishOperation::Update => {
            let id = match request
                .existing_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                Some(id) => id.to_string(),
                None => {
                    return PublishResult::failure(
                        FailureClass::NotConfigured,
                        format!("{}更新草稿时缺少草稿 ID，请重新同步一次", PLATFORM),
                        operation,
                        false,
                    )
                }
            };

            // 更新必须带上旧 pgc_id 重新提交，否则草稿内容不会真正覆盖；
            // 之前这里只取 ID 就跳出，等于「假成功」。
            let value = match submit_article(
                &client,
                base,
                cookie,
                &csrf_token,
                request,
                &html,
                &id,
                false,
                operation,
            )
            .await
            {
                Ok(value) => value,
                Err(failure) => return failure,
            };

            // 更新响应可能不带 pgc_id，此时沿用提交的 ID；若返回了一个不同的 ID，
            // 说明服务端把这次更新当成新建了，静默采纳会污染同步表，必须报错。
            match article_id(&value) {
                Some(returned) if returned != id => {
                    return response::business_failure(
                        PLATFORM,
                        format!(
                            "草稿更新结果不一致：提交的 ID 为 {}，服务端返回 {}，请到头条后台确认",
                            id, returned
                        ),
                        operation,
                        false,
                    )
                }
                _ => id,
            }
        }
        PublishOperation::Create => {
            let value = match submit_article(
                &client,
                base,
                cookie,
                &csrf_token,
                request,
                &html,
                "0",
                false,
                operation,
            )
            .await
            {
                Ok(value) => value,
                Err(failure) => return failure,
            };

            match article_id(&value) {
                Some(id) => id,
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

    let mut is_published = request.existing_published;
    let mut post_id = draft_id.clone();

    if request.publish_status && !request.existing_published {
        let value = match submit_article(
            &client,
            base,
            cookie,
            &csrf_token,
            request,
            &html,
            &draft_id,
            true,
            operation,
        )
        .await
        {
            Ok(value) => value,
            Err(failure) => return failure,
        };

        let published_id = match article_id(&value) {
            Some(id) => id,
            None => {
                return response::business_failure(
                    PLATFORM,
                    "发布结果不明: 响应缺少文章 ID",
                    operation,
                    false,
                )
            }
        };

        if published_id != draft_id {
            return response::business_failure(
                PLATFORM,
                "发布结果与草稿 ID 不一致，请在头条后台确认发布状态",
                operation,
                false,
            );
        }

        post_id = published_id;
        is_published = true;
    }

    let edit_url = format!("{}?pgc_id={}", EDITOR_URL, draft_id);
    let message = if is_published {
        format!("已成功发布到今日头条！文章 ID: {}", post_id)
    } else {
        format!("今日头条草稿已保存！草稿 ID: {}", draft_id)
    };

    PublishResult::success(post_id, edit_url, message, operation, is_published)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::MockServer;

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "头条标题".to_string(),
            content: "## 正文\n\n第一行".to_string(),
            publish_status,
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "sessionid=abc; ttwid=def",
            request,
            operation,
            server_base,
        ))
    }

    fn csrf_server() -> MockServer {
        MockServer::start_with_headers(
            vec![
                ("HEAD /ttwid/check/", 200, ""),
                (
                    "POST /mp/agw/article/publish",
                    200,
                    r#"{"err_no":0,"message":"success","data":{"pgc_id":"7001"}}"#,
                ),
            ],
            (500, "{}"),
            &[("x-ware-csrf-token", "csrf-token-whole")],
        )
    }

    #[test]
    fn success_saves_draft_and_publishes() {
        let server = csrf_server();

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "7001");
        assert!(result.is_published);
        assert!(result.edit_url.contains("pgc_id=7001"));
    }

    #[test]
    fn draft_only_keeps_draft_id() {
        let server = csrf_server();

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "7001");
        assert!(!result.is_published);
    }

    #[test]
    fn probe_failure_is_not_configured() {
        let server = MockServer::start(vec![("HEAD /ttwid/check/", 200, "")], (500, "{}"));

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NotConfigured));
        assert!(result.message.contains("x-ware-csrf-token"));
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start_with_headers(
            vec![("HEAD /ttwid/check/", 200, "")],
            (403, "{}"),
            &[("x-ware-csrf-token", "csrf-token-whole")],
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn update_submits_draft_and_reuses_id_when_response_omits_it() {
        let server = MockServer::start_with_headers(
            vec![
                ("HEAD /ttwid/check/", 200, ""),
                (
                    "POST /mp/agw/article/publish",
                    200,
                    r#"{"err_no":0,"message":"success"}"#,
                ),
            ],
            (500, "{}"),
            &[("x-ware-csrf-token", "csrf-token-whole")],
        );

        let mut request = sample_request(false);
        request.existing_id = Some("7001".to_string());

        let result = run(&server.base, &request, PublishOperation::Update);

        // 更新必须真正提交并复用旧 pgc_id，而不是只取 ID 就返回「假成功」。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "7001");
        assert!(result.edit_url.contains("pgc_id=7001"));
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start_with_headers(
            vec![
                ("HEAD /ttwid/check/", 200, ""),
                (
                    "POST /mp/agw/article/publish",
                    200,
                    r#"{"err_no":1001,"message":"标题不合法"}"#,
                ),
            ],
            (500, "{}"),
            &[("x-ware-csrf-token", "csrf-token-whole")],
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("标题不合法"));
    }

    #[test]
    fn update_returning_different_id_is_platform_error() {
        let server = MockServer::start_with_headers(
            vec![
                ("HEAD /ttwid/check/", 200, ""),
                (
                    "POST /mp/agw/article/publish",
                    200,
                    r#"{"err_no":0,"message":"success","data":{"pgc_id":"8888"}}"#,
                ),
            ],
            (500, "{}"),
            &[("x-ware-csrf-token", "csrf-token-whole")],
        );

        let mut request = sample_request(true);
        request.existing_id = Some("7001".to_string());

        let result = run(&server.base, &request, PublishOperation::Update);

        // 提交 7001 却返回 8888，说明服务端当成新建了，必须报错而不是静默采纳新 ID。
        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("不一致"));
        assert!(result.message.contains("7001"), "{}", result.message);
        assert!(result.message.contains("8888"), "{}", result.message);
    }
}