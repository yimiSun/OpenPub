use base64::Engine;
use serde_json::Value;

use super::shared::{cookie, extract, markdown, page_token, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "weibo_config";
const PLATFORM: &str = "新浪微博";
const BASE_URL: &str = "https://card.weibo.com";
const EDITOR_PAGE_PATH: &str = "/article/v5/editor";
const SUCCESS_CODE: i64 = 100_000;

/// 微博把编辑器配置写在页面里的 `config:JSON.parse('...')`，其中 `uid` 是后续所有
/// 草稿接口都必须带上的用户标识，等价于「预检」：拿不到就说明 Cookie 不可用。
const TOKEN_RULE: page_token::TokenRule = page_token::TokenRule {
    regex: r#"config:\s*JSON\.parse\('(.+?)'\)"#,
    json_path: Some("uid"),
};

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "新浪微博不支持修改已发布的文章，请先在微博后台将其转为草稿后再同步".to_string(),
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

/// 微博的 `_rid` 是请求追踪号，服务端不校验内容，但要求长度稳定。
fn request_id(uid: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let seed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!("{}&{}", uid, millis));
    let filler = uuid::Uuid::new_v4().simple().to_string();

    let mut id = seed;
    for byte in filler.bytes() {
        if id.len() >= 43 {
            break;
        }
        id.push(byte as char);
    }
    id.chars().take(43).collect()
}

fn business_error(value: &Value, fallback: &str) -> String {
    value
        .get("msg")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn success_code(value: &Value) -> bool {
    extract::read_numeric_field(value, &["code"]) == Some(SUCCESS_CODE)
}

fn save_form(
    request: &PublishRequest,
    html: &str,
    draft_id: &str,
    resave: bool,
    rid: &str,
) -> Vec<(&'static str, String)> {
    let mut form: Vec<(&'static str, String)> = vec![
        ("id", draft_id.to_string()),
        ("title", request.title.clone()),
        ("subtitle", String::new()),
        ("type", String::new()),
        ("status", "0".to_string()),
        ("publish_at", String::new()),
        ("error_msg", String::new()),
        ("error_code", "0".to_string()),
        ("collection", "[]".to_string()),
        ("free_content", String::new()),
        ("content", html.to_string()),
        ("cover", String::new()),
        ("summary", String::new()),
        ("writer", String::new()),
        (
            "extra",
            if resave { "[]" } else { "null" }.to_string(),
        ),
        ("is_word", "0".to_string()),
        ("is_markdown", "0".to_string()),
        (
            "article_recommend",
            if resave { "{}" } else { "[]" }.to_string(),
        ),
        ("follow_to_read", "1".to_string()),
        ("isreward", "1".to_string()),
        (
            "pay_setting",
            if resave {
                r#"{"ispay":0,"isvclub":0,"is_single_pay":0,"single_price":0}"#
            } else {
                r#"{"ispay":0,"isvclub":0}"#
            }
            .to_string(),
        ),
        ("source", if resave { "" } else { "0" }.to_string()),
        ("action", if resave { "2" } else { "1" }.to_string()),
        ("content_type", "0".to_string()),
        ("save", "1".to_string()),
    ];

    if resave {
        form.extend(vec![
            ("publish_local_at", String::new()),
            ("timestamp", String::new()),
            ("is_article_free", "0".to_string()),
            ("only_render_h5", "0".to_string()),
            ("is_ai_plugins", "0".to_string()),
            ("is_aigc_used", "0".to_string()),
            ("is_v4", "1".to_string()),
            ("is_single_pay_new", "0".to_string()),
            ("money", "0".to_string()),
            ("is_vclub_single_pay", "0".to_string()),
            ("vclub_single_pay_money", "0".to_string()),
            ("ver", "4.0".to_string()),
            ("_rid", rid.to_string()),
        ]);
    }

    form
}

fn publish_form(title: &str, draft_id: &str, uid: &str) -> Vec<(&'static str, String)> {
    vec![
        ("rank", "0".to_string()),
        ("mblog_statement", "0".to_string()),
        ("sync_wb", "0".to_string()),
        ("is_original", "0".to_string()),
        ("follow_official", "0".to_string()),
        ("text", format!("发布了头条文章：《{}》 ", title)),
        ("time", String::new()),
        ("id", draft_id.to_string()),
        ("timestamp", String::new()),
        ("only_render_h5", "0".to_string()),
        ("is_aigc_used", "0".to_string()),
        ("follow_to_read", "1".to_string()),
        ("mpkey", "0".to_string()),
        ("uid", uid.to_string()),
        ("ver", "4.0".to_string()),
        ("support_all_tag", "1".to_string()),
    ]
}

async fn create_draft(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    uid: &str,
    operation: PublishOperation,
) -> Result<String, PublishResult> {
    let rid = request_id(uid);
    let url = format!(
        "{}/article/v5/aj/editor/draft/create?uid={}&_rid={}",
        base, uid, rid
    );

    let (status, body) = response::read_text(
        client
            .post(&url)
            .header("Cookie", cookie)
            .header("Accept", "application/json, text/plain, */*")
            .header("SN-REQID", &rid)
            .header("Origin", "https://card.weibo.com")
            .header(
                "Referer",
                format!("{}{}", base, EDITOR_PAGE_PATH),
            )
            .header("User-Agent", response::USER_AGENT)
            .form(&Vec::<(&str, String)>::new())
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
                "创建草稿失败: 响应不是有效 JSON",
                operation,
                false,
            ))
        }
    };

    if !success_code(&value) {
        let detail = business_error(&value, "创建新浪微博草稿失败");
        return Err(response::business_failure(PLATFORM, detail, operation, false));
    }

    match value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["id"]))
    {
        Some(id) => Ok(id),
        None => Err(response::business_failure(
            PLATFORM,
            "创建草稿失败: 响应缺少草稿 ID",
            operation,
            false,
        )),
    }
}

async fn save_draft(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    uid: &str,
    request: &PublishRequest,
    draft_id: &str,
    resave: bool,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let rid = request_id(uid);
    let url = format!(
        "{}/article/v5/aj/editor/draft/save?uid={}&id={}&_rid={}",
        base, uid, draft_id, rid
    );
    let html = markdown::to_html_content(&request.content);
    let form = save_form(request, &html, draft_id, resave, &rid);

    let (status, body) = response::read_text(
        client
            .post(&url)
            .header("Cookie", cookie)
            .header("Accept", "application/json, text/plain, */*")
            .header("SN-REQID", &rid)
            .header("Origin", "https://card.weibo.com")
            .header(
                "Referer",
                format!("{}{}", base, EDITOR_PAGE_PATH),
            )
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
                "保存草稿失败: 响应不是有效 JSON",
                operation,
                false,
            ))
        }
    };

    if !success_code(&value) {
        let detail = business_error(&value, "保存新浪微博草稿失败");
        return Err(response::business_failure(PLATFORM, detail, operation, false));
    }

    let verify_url = value
        .get("data")
        .and_then(|data| data.get("verifyUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty());

    if let Some(verify_url) = verify_url {
        return Err(PublishResult::failure(
            FailureClass::NotImplemented,
            format!(
                "{} 要求先完成人机验证（{}），请在微博编辑页完成验证后重试",
                PLATFORM,
                response::snippet(verify_url)
            ),
            operation,
            false,
        ));
    }

    Ok(())
}

async fn submit_draft(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    uid: &str,
    request: &PublishRequest,
    draft_id: &str,
    operation: PublishOperation,
) -> Result<(String, String), PublishResult> {
    let rid = request_id(uid);
    let url = format!(
        "{}/article/v5/aj/editor/draft/publish?uid={}&id={}&_rid={}",
        base, uid, draft_id, rid
    );
    let form = publish_form(&request.title, draft_id, uid);

    let (status, body) = response::read_text(
        client
            .post(&url)
            .header("Cookie", cookie)
            .header("Accept", "application/json, text/plain, */*")
            .header("SN-REQID", &rid)
            .header("Origin", "https://card.weibo.com")
            .header(
                "Referer",
                format!("{}{}", base, EDITOR_PAGE_PATH),
            )
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
                "发布响应不是有效 JSON，无法确认是否发布成功",
                operation,
                false,
            ))
        }
    };

    if !success_code(&value) {
        let detail = business_error(&value, "新浪微博拒绝发布请求");
        return Err(response::business_failure(PLATFORM, detail, operation, false));
    }

    let article_url = value
        .get("data")
        .and_then(|data| data.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty());

    let article_url = match article_url {
        Some(url) => url.to_string(),
        None => {
            return Err(response::business_failure(
                PLATFORM,
                "发布结果不明: 响应缺少文章地址",
                operation,
                false,
            ))
        }
    };

    let post_id = value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["object_id"]))
        .unwrap_or_else(|| draft_id.to_string());

    Ok((post_id, article_url))
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    base: &str,
) -> PublishResult {
    let client = response::new_client();
    let editor_page = format!("{}{}", base, EDITOR_PAGE_PATH);

    let uid = match page_token::fetch_token_with_cookie(
        &client,
        &editor_page,
        Some(cookie),
        &TOKEN_RULE,
    )
    .await
    {
        Ok(uid) => uid,
        Err(error) => {
            return PublishResult::failure(
                FailureClass::NotConfigured,
                format!(
                    "{}未取得编辑器配置（{}），请重新登录或更新 Cookie",
                    PLATFORM, error
                ),
                operation,
                false,
            )
        }
    };

    let draft_id = match operation {
        PublishOperation::Update => {
            match request
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
            }
        }
        PublishOperation::Create => {
            match create_draft(&client, base, cookie, &uid, operation).await {
                Ok(id) => id,
                Err(failure) => return failure,
            }
        }
    };

    let resave = operation == PublishOperation::Update;
    if let Err(failure) = save_draft(
        &client, base, cookie, &uid, request, &draft_id, resave, operation,
    )
    .await
    {
        return failure;
    }

    let mut is_published = request.existing_published;
    let mut post_id = draft_id.clone();
    let mut article_url = String::new();

    if request.publish_status && !request.existing_published {
        match submit_draft(&client, base, cookie, &uid, request, &draft_id, operation).await {
            Ok((id, url)) => {
                post_id = id;
                article_url = url;
                is_published = true;
            }
            Err(failure) => return failure,
        }
    }

    let edit_url = if article_url.is_empty() {
        format!("{}{}#/draft/{}", base, EDITOR_PAGE_PATH, draft_id)
    } else {
        article_url
    };

    let message = if is_published {
        format!("新浪微博文章已发布！文章 ID: {}", post_id)
    } else {
        format!("新浪微博草稿已保存！草稿 ID: {}", draft_id)
    };

    PublishResult::success(post_id, edit_url, message, operation, is_published)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::MockServer;

    const EDITOR_PAGE: &str = r#"<html><script>config:JSON.parse('{"uid":"1234567","nick":"测试"}')</script></html>"#;

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "微博标题".to_string(),
            content: "## 正文\n\n第一行".to_string(),
            publish_status,
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "SUB=abc; SUBP=def",
            request,
            operation,
            server_base,
        ))
    }

    #[test]
    fn success_creates_saves_and_publishes() {
        let server = MockServer::start(
            vec![
                ("GET /article/v5/editor", 200, EDITOR_PAGE),
                (
                    "POST /article/v5/aj/editor/draft/create",
                    200,
                    r#"{"code":100000,"data":{"id":"9001"}}"#,
                ),
                (
                    "POST /article/v5/aj/editor/draft/save",
                    200,
                    r#"{"code":100000,"data":{}}"#,
                ),
                (
                    "POST /article/v5/aj/editor/draft/publish",
                    200,
                    r#"{"code":100000,"data":{"object_id":"777","url":"https://weibo.com/ttarticle/p/show?id=777"}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "777");
        assert!(result.is_published);
        assert!(result.edit_url.contains("show?id=777"));
    }

    #[test]
    fn draft_only_keeps_draft_id() {
        let server = MockServer::start(
            vec![
                ("GET /article/v5/editor", 200, EDITOR_PAGE),
                (
                    "POST /article/v5/aj/editor/draft/create",
                    200,
                    r#"{"code":100000,"data":{"id":"9001"}}"#,
                ),
                (
                    "POST /article/v5/aj/editor/draft/save",
                    200,
                    r#"{"code":100000,"data":{}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9001");
        assert!(!result.is_published);
        assert!(result.edit_url.contains("#/draft/9001"));
    }

    #[test]
    fn missing_editor_config_is_not_configured() {
        let server = MockServer::start(
            vec![("GET /article/v5/editor", 200, "<html>请登录</html>")],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::NotConfigured));
        assert!(result.message.contains("未取得编辑器配置"));
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(
            vec![("GET /article/v5/editor", 200, EDITOR_PAGE)],
            (403, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn platform_error_maps_to_platform_error() {
        let server = MockServer::start(
            vec![
                ("GET /article/v5/editor", 200, EDITOR_PAGE),
                (
                    "POST /article/v5/aj/editor/draft/create",
                    200,
                    r#"{"code":-100,"msg":"创建草稿失败"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("创建草稿失败"));
    }
}