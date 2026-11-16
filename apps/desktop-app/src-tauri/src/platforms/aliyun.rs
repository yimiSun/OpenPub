use serde_json::{json, Value};

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "aliyun_config";
const PLATFORM: &str = "阿里云开发者社区";
const BASE_URL: &str = "https://developer.aliyun.com";
const CSRF_COOKIE_KEY: &str = "c_csrf";
/// `c_csrf` 的备选键名。阿里云在不同子域下发的令牌键名不一致：
/// `developer.aliyun.com` 给的是 `c_csrf`，统一登录域（`account.aliyun.com`）
/// 下给的是 `csrf` 或 `XSRF-TOKEN`。抓 Cookie 时把业务域并进列表后，
/// 三者可能同时出现，取到任意一个都能通过网关校验。
const CSRF_COOKIE_KEYS: &[&str] = &["c_csrf", "csrf", "XSRF-TOKEN", "XSRF-TOKEN-ALIYUN"];

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

/// 依次尝试所有候选键名取 CSRF 令牌，同时返回命中的键名，便于日志定位来源。
fn resolve_csrf(cookie: &str) -> Result<(&'static str, String), String> {
    for key in CSRF_COOKIE_KEYS {
        if let Some(value) = cookie::extract_value(cookie, key) {
            return Ok((*key, value));
        }
    }
    Err(format!(
        "Cookie 中缺少 {}，请重新登录 {} 后复制完整 Cookie",
        CSRF_COOKIE_KEY, PLATFORM
    ))
}

fn log_csrf_source(key: &str) {
    if key == CSRF_COOKIE_KEY {
        eprintln!("[aliyun] CSRF 来源 {}", key);
    } else {
        eprintln!(
            "[aliyun] CSRF 来源 {}（非 {}，网关校验可能不认这个别名）",
            key, CSRF_COOKIE_KEY
        );
    }
}

/// 把 Cookie 里已存在的 CSRF 别名统一成同一个令牌值。
///
/// 抓取凭据时会把 `login.aliyun.com` / `account.aliyun.com` / `developer.aliyun.com`
/// 三个域的 Cookie 合并成一个串，同一个令牌可能以 `c_csrf` 与 `csrf` 两个键名
/// 出现且值不同。网关若按别名取值、而我们用另一个值发 `h_csrf`，请求就会被判
/// 成 CSRF 不匹配——线上表现正是「接口返回成功、草稿里却是空的」。
/// 这里只改写已存在的别名，不新增字段，避免塞入平台没发过的 Cookie。
fn align_csrf_cookies(cookie: &str, token: &str) -> String {
    let mut aligned = cookie.to_string();
    for key in CSRF_COOKIE_KEYS {
        if *key == CSRF_COOKIE_KEY {
            continue;
        }
        if let Some(value) = cookie::extract_value(&aligned, key) {
            if value != token {
                aligned = upsert_cookie(&aligned, key, token);
                eprintln!("[aliyun] CSRF 别名 {} 与 {} 不一致，已对齐", key, CSRF_COOKIE_KEY);
            }
        }
    }
    aligned
}

/// 解析出可用的 CSRF 令牌，并尽量把业务域的 `c_csrf` 补上。
///
/// 优先级：本地已有的 `c_csrf` > 访问首页补抓到的 `c_csrf` > 其它别名。
/// 只有别名时也先补抓一次，因为网关校验认的是业务域下发的那个令牌。
async fn prepare_csrf(
    client: &reqwest::Client,
    base: &str,
    cookie: &str,
    operation: PublishOperation,
) -> Result<(String, String), PublishResult> {
    let mut merged = cookie.to_string();

    if let Ok((key, token)) = resolve_csrf(&merged) {
        if key == CSRF_COOKIE_KEY {
            log_csrf_source(key);
            return Ok((merged, token));
        }
    }

    if let Some(probed) = probe_csrf_cookie(client, base, &merged).await {
        merged = probed;
    }

    match resolve_csrf(&merged) {
        Ok((key, token)) => {
            log_csrf_source(key);
            Ok((merged, token))
        }
        Err(message) => Err(response::auth_failure(PLATFORM, &message, operation, false)),
    }
}

/// 把 `Set-Cookie` 解析出的键值并进已有 Cookie 串：同名覆盖，缺失追加。
fn upsert_cookie(cookie: &str, name: &str, value: &str) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut replaced = false;

    for segment in cookie.split(';') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.split_once('=') {
            Some((key, _)) if key.trim() == name => {
                if !replaced {
                    segments.push(format!("{}={}", name, value));
                    replaced = true;
                }
            }
            _ => segments.push(trimmed.to_string()),
        }
    }

    if !replaced {
        segments.push(format!("{}={}", name, value));
    }

    segments.join("; ")
}

/// 用已有 Cookie 访问一次首页，从 `Set-Cookie` 里回收缺失的 CSRF 令牌。
///
/// 抓取凭据时通常只存下了统一登录域（`account.aliyun.com`）的 Cookie，
/// 而网关校验用的 `c_csrf` 是 `developer.aliyun.com` 访问页面后才下发的。
/// 这里只合并 `CSRF_COOKIE_KEYS` 里的键，不动其余登录态字段——首页可能是
/// 半登录页，全量覆盖会把仍然有效的会话 Cookie 冲掉。
async fn probe_csrf_cookie(client: &reqwest::Client, base: &str, cookie: &str) -> Option<String> {
    let response = client
        .get(format!("{}/", base))
        .header("Cookie", cookie)
        .header("Accept", "text/html,application/xhtml+xml,*/*")
        .header("User-Agent", response::USER_AGENT)
        .send()
        .await
        .ok()?;

    let mut merged = cookie.to_string();
    let mut touched = false;

    for value in response.headers().get_all("set-cookie").iter() {
        let raw = match value.to_str() {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let pair = raw.split(';').next().unwrap_or("").trim();
        let (name, token) = match pair.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let name = name.trim();
        let token = token.trim();
        if token.is_empty() || !CSRF_COOKIE_KEYS.contains(&name) {
            continue;
        }
        merged = upsert_cookie(&merged, name, token);
        touched = true;
    }

    if touched {
        Some(merged)
    } else {
        None
    }
}

/// 决定 `putDraft` 的 `aid`：只有更新操作且旧 ID 能解析成数字时才带上。
///
/// 新建路径必须留空，否则阿里云会拿一个不存在的 ID 去查，反而报错。
fn resolve_aid(request: &PublishRequest, operation: PublishOperation) -> Option<i64> {
    if operation != PublishOperation::Update {
        return None;
    }
    request
        .existing_id
        .as_deref()
        .and_then(|id| id.trim().parse::<i64>().ok())
}

/// 从 `putDraft` 响应里取草稿 ID，并返回命中的键名。
///
/// 线上只见过 `data.aid`，但平台改字段名时只认 `aid` 会让我们把「已保存的草稿」
/// 误报成「响应缺少草稿 ID」。这里按候选键名依次尝试，命中哪个都记一条日志，
/// 真机抓包时能直接看出响应结构。
const DRAFT_ID_KEYS: &[&str] = &["draftId", "aid", "id", "articleId"];

fn draft_id_from_response(value: &Value) -> Option<(String, &'static str)> {
    let data = value.get("data")?;
    for key in DRAFT_ID_KEYS {
        if let Some(id) = extract::extract_id(data, &[*key]) {
            return Some((id, *key));
        }
    }
    None
}

/// 构造 `putDraft` 请求体。
///
/// 字段名与平台编辑器自己发出的请求逐字一致（`title` / `content` /
/// `contentRender` / `format` / `aid` / `productTags` / `freeTierVOS`）。
/// `content` 必须是 Markdown 源码：阿里云编辑器按 Markdown 解析，传 HTML 会把
/// 标签原样显示。`contentRender` 保持空串、`format` 保持 `null`，与编辑器行为
/// 一致，由服务端自行渲染；不要擅自填 HTML，否则编辑器会把渲染结果当源码再解析一遍。
fn build_draft_payload(request: &PublishRequest, aid: Option<i64>) -> Value {
    json!({
        "title": request.title,
        "content": request.content,
        "contentRender": "",
        "format": Value::Null,
        "aid": aid.map(Value::from).unwrap_or(Value::Null),
        "productTags": [],
        "freeTierVOS": [],
    })
}

/// 打印真实请求体：字段名 + 长度 + 正文前 120 字。
///
/// 「接口成功但草稿是空的」这类问题必须先看发出去的是什么，否则只能靠猜。
fn log_draft_request(url: &str, payload: &Value) {
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
    let content = payload.get("content").and_then(Value::as_str).unwrap_or("");
    let keys = payload
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    let preview: String = content.chars().take(120).collect();
    eprintln!(
        "[aliyun] 草稿请求 POST {} keys=[{}] title_len={} content_len={} content_preview={:?}",
        url,
        keys,
        title.chars().count(),
        content.chars().count(),
        preview
    );
    eprintln!("[aliyun] 草稿请求 body={}", payload);
}

fn log_draft_response(body: &str) {
    let preview: String = body.chars().take(600).collect();
    eprintln!("[aliyun] 草稿响应 body={}", preview);
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.publish_status && !request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "阿里云开发者社区的正式发布依赖编辑器页面内的提交动作，当前仅支持保存草稿".to_string(),
        ));
    }
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "阿里云开发者社区没有草稿更新接口，已发布文章的同步更新暂不支持，请在后台手动修改"
                .to_string(),
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
    // 正文为空时平台照样会「成功」建出一份空草稿，所以在这里直接拦下，
    // 不要把它交给服务端变成一份看起来正常、打开却什么都没有的草稿。
    if request.title.trim().is_empty() || request.content.trim().is_empty() {
        return response::business_failure(
            PLATFORM,
            "草稿未创建：标题或正文为空，阿里云会把空内容存成空草稿",
            operation,
            false,
        );
    }

    let client = response::new_client();

    // 凭据里往往只有统一登录域的 Cookie，业务域的 `c_csrf` 要访问页面才下发。
    // 先在本地解析，缺了再去首页补抓一次，避免用户被迫重新扫码。
    let (cookie, csrf) = match prepare_csrf(&client, base, cookie, operation).await {
        Ok(pair) => pair,
        Err(failure) => return failure,
    };

    // 网关按 Cookie 里的某个 CSRF 别名取值校验，三域 Cookie 合并后同名令牌可能
    // 出现两个不同值；统一成发出去的这一个，避免被判 CSRF 不匹配而丢掉正文。
    let cookie = align_csrf_cookies(&cookie, &csrf);
    let cookie = cookie.as_str();

    let referer = format!("{}/article/new", base);

    let user_url = format!("{}/developer/api/my/user/getUser", base);
    let (status, body) = match response::read_text(
        client
            .get(&user_url)
            .header("Cookie", cookie)
            .header("h_csrf", &csrf)
            .header("X-XSRF-TOKEN", &csrf)
            .header("x-csrf-token", &csrf)
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Origin", base)
            .header("Referer", &referer)
            .header("Sec-Fetch-Site", "same-origin")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Dest", "empty")
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
                "检查登录状态失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let detail = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未登录，请先登录阿里云开发者社区");
        return response::auth_failure(PLATFORM, detail, operation, false);
    }

    let user_id = value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["uccId"]))
        .unwrap_or_default();
    if user_id.is_empty() {
        return response::auth_failure(
            PLATFORM,
            "未登录，请先登录阿里云开发者社区",
            operation,
            false,
        );
    }

    let draft_url = format!("{}/developer/api/articleDraft/putDraft?p_csrf={}", base, csrf);

    // `putDraft` 用请求体里的 `aid` 区分新建与更新：空值新建，已有 ID 覆盖。
    // 不传就会每次生成一份新草稿（线上表现为 1711678 -> 1712629 这种跳号）。
    let existing_aid = resolve_aid(request, operation);

    let payload = build_draft_payload(request, existing_aid);
    log_draft_request(&draft_url, &payload);

    let (status, body) = match response::read_text(
        client
            .post(&draft_url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/json")
            .header("h_csrf", &csrf)
            .header("X-XSRF-TOKEN", &csrf)
            .header("x-csrf-token", &csrf)
            .header("Accept", "application/json, text/plain, */*")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Origin", base)
            .header("Referer", &referer)
            .header("Sec-Fetch-Site", "same-origin")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Dest", "empty")
            .header("User-Agent", response::USER_AGENT)
            .json(&payload)
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

    log_draft_response(&body);

    let value = match response::parse_json(&body) {
        Some(value) => value,
        None => {
            return response::business_failure(
                PLATFORM,
                "保存草稿失败: 响应不是有效 JSON",
                operation,
                false,
            )
        }
    };

    let draft_id = match draft_id_from_response(&value) {
        Some((id, key)) => {
            eprintln!("[aliyun] 草稿 ID 取自响应字段 data.{}", key);
            id
        }
        None => {
            // 更新时响应可能只回 `success:true` 而不带 aid，此时复用刚提交的旧 ID。
            // 用 `existing_aid` 而非 `request.existing_id`：前者已校验过「确实是更新
            // 且能解析成数字」，不会把新建路径上的空值或非法串当成草稿 ID。
            if let Some(aid) = existing_aid {
                aid.to_string()
            } else {
                let detail = value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("保存草稿失败: 响应缺少草稿 ID");
                return response::business_failure(PLATFORM, detail, operation, false);
            }
        }
    };

    let edit_url = format!("{}/article/new?edit={}#/", base, draft_id);
    let message = format!("阿里云开发者社区草稿已保存！草稿 ID: {}", draft_id);

    PublishResult::success(draft_id, edit_url, message, operation, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    fn sample_request() -> PublishRequest {
        PublishRequest {
            title: "阿里云标题".to_string(),
            content: "# 正文".to_string(),
            ..Default::default()
        }
    }

    fn run(server_base: &str, request: &PublishRequest, operation: PublishOperation) -> PublishResult {
        tauri::async_runtime::block_on(publish_with_cookie(
            "c_csrf=tok123; other=1",
            request,
            operation,
            server_base,
        ))
    }

    #[test]
    fn success_returns_draft_id() {
        let server = MockServer::start(
            vec![
                (
                    "GET /developer/api/my/user/getUser",
                    200,
                    r#"{"success":true,"data":{"uccId":"5566"}}"#,
                ),
                (
                    "POST /developer/api/articleDraft/putDraft",
                    200,
                    r#"{"success":true,"data":{"aid":"7007"}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "7007");
        assert!(!result.is_published);
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
        let server = MockServer::start(vec![], (500, r#"{"message":"服务异常"}"#));

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
    fn missing_csrf_cookie_is_auth_invalid() {
        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "session=abc",
            &sample_request(),
            PublishOperation::Create,
            &closed_port_base(),
        ));

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }

    #[test]
    fn csrf_token_is_resolved_from_alternate_cookie_names() {
        assert_eq!(resolve_csrf("c_csrf=tok").unwrap().1, "tok");
        assert_eq!(resolve_csrf("c_csrf=tok").unwrap().0, "c_csrf");
        assert_eq!(resolve_csrf("csrf=tok").unwrap(), ("csrf", "tok".to_string()));
        assert_eq!(
            resolve_csrf("XSRF-TOKEN=tok").unwrap(),
            ("XSRF-TOKEN", "tok".to_string())
        );
        assert_eq!(
            resolve_csrf("other=1; csrf=abc").unwrap(),
            ("csrf", "abc".to_string())
        );
        assert!(resolve_csrf("session=abc").is_err());
    }

    #[test]
    fn csrf_alias_values_are_aligned_to_header_token() {
        // 三域 Cookie 合并后同名令牌可能有两个值，网关按别名取值就会判不匹配。
        let aligned = align_csrf_cookies("c_csrf=tok; csrf=other; XSRF-TOKEN=x", "tok");
        assert_eq!(aligned, "c_csrf=tok; csrf=tok; XSRF-TOKEN=tok");

        // 没有别名时不得凭空塞入平台没下发过的 Cookie。
        let untouched = align_csrf_cookies("a=1; c_csrf=tok", "tok");
        assert_eq!(untouched, "a=1; c_csrf=tok");
    }

    #[test]
    fn payload_keeps_markdown_source_and_matches_editor_fields() {
        let payload = build_draft_payload(&sample_request(), None);

        // 正文必须原样传 Markdown 源码，不能转成 HTML。
        assert_eq!(payload.get("content").and_then(Value::as_str), Some("# 正文"));
        assert_eq!(
            payload.get("title").and_then(Value::as_str),
            Some("阿里云标题")
        );
        assert_eq!(
            payload.get("contentRender").and_then(Value::as_str),
            Some("")
        );
        assert!(payload.get("format").map(Value::is_null).unwrap_or(false));
        assert!(payload.get("aid").map(Value::is_null).unwrap_or(false));
        assert_eq!(
            payload.get("productTags").and_then(Value::as_array).map(Vec::len),
            Some(0)
        );
        assert_eq!(
            payload.get("freeTierVOS").and_then(Value::as_array).map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn payload_field_names_match_v100_editor_contract() {
        // 契约来源：阿里云编辑器自身发出的请求（与 v1.0.0 快照逐字一致）。
        // 「markdown / categoryId / tagIds」从来不是该端点的字段；若哪天出现，
        // 说明字段名改漂移了，是回归信号。若平台未来真的换了结构，正确动作是
        // 按新抓包改 build_draft_payload，并同步更新本测试，而不是只删断言。
        let payload = build_draft_payload(&sample_request(), None);

        let actual: std::collections::HashSet<&str> = payload
            .as_object()
            .expect("payload 是对象")
            .keys()
            .map(String::as_str)
            .collect();
        let expected: std::collections::HashSet<&str> = [
            "title",
            "content",
            "contentRender",
            "format",
            "aid",
            "productTags",
            "freeTierVOS",
        ]
        .into_iter()
        .collect();
        assert_eq!(actual, expected);

        // 三键显式断言，失败信息直接指向契约来源。
        assert!(payload.get("markdown").is_none(), "markdown 不是阿里云端点字段，content 才是");
        assert!(payload.get("categoryId").is_none(), "categoryId 不是阿里云端点字段");
        assert!(payload.get("tagIds").is_none(), "tagIds 不是阿里云端点字段");
    }

    #[test]
    fn payload_carries_aid_for_update_only() {
        let mut request = sample_request();
        request.existing_id = Some("1711678".to_string());

        let payload = build_draft_payload(&request, resolve_aid(&request, PublishOperation::Update));
        assert_eq!(payload.get("aid").and_then(Value::as_i64), Some(1711678));

        let payload = build_draft_payload(&request, resolve_aid(&request, PublishOperation::Create));
        assert!(payload.get("aid").map(Value::is_null).unwrap_or(false));
    }

    #[test]
    fn draft_id_prefers_platform_native_field() {
        let value: Value = serde_json::from_str(r#"{"success":true,"data":{"draftId":"7007","aid":"1"}}"#).unwrap();
        assert_eq!(
            draft_id_from_response(&value),
            Some(("7007".to_string(), "draftId"))
        );

        let value: Value = serde_json::from_str(r#"{"success":true,"data":{"aid":"7007"}}"#).unwrap();
        assert_eq!(
            draft_id_from_response(&value),
            Some(("7007".to_string(), "aid"))
        );

        let value: Value = serde_json::from_str(r#"{"success":true}"#).unwrap();
        assert_eq!(draft_id_from_response(&value), None);
    }

    #[test]
    fn empty_content_is_rejected_before_request() {
        // 空正文交给平台会「成功」建出一份空草稿，必须本地拦下并归类为业务失败，
        // 而不是网络错误——后者会误导用户以为没连上网。
        let mut request = sample_request();
        request.content = "   ".to_string();

        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "c_csrf=tok123",
            &request,
            PublishOperation::Create,
            &closed_port_base(),
        ));

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("标题或正文为空"), "{}", result.message);
    }

    #[test]
    fn upsert_cookie_replaces_existing_key_and_keeps_order() {
        let merged = upsert_cookie("a=1; c_csrf=OLD; b=2", "c_csrf", "NEW");
        assert_eq!(merged, "a=1; c_csrf=NEW; b=2");
    }

    #[test]
    fn update_reuses_existing_aid_instead_of_creating_new_draft() {
        let mut request = sample_request();
        request.existing_id = Some("1711678".to_string());

        // 更新操作必须把旧 aid 带上，否则阿里云每次都会新建草稿。
        assert_eq!(resolve_aid(&request, PublishOperation::Update), Some(1711678));

        // 新建操作即使带了 existing_id 也不能传，避免拿旧 ID 去覆盖。
        assert_eq!(resolve_aid(&request, PublishOperation::Create), None);
    }

    #[test]
    fn non_numeric_or_missing_aid_falls_back_to_null() {
        let mut request = sample_request();
        request.existing_id = Some("not-a-number".to_string());
        assert_eq!(resolve_aid(&request, PublishOperation::Update), None);

        request.existing_id = Some("   ".to_string());
        assert_eq!(resolve_aid(&request, PublishOperation::Update), None);

        request.existing_id = None;
        assert_eq!(resolve_aid(&request, PublishOperation::Update), None);
    }

    #[test]
    fn update_without_aid_in_response_reuses_submitted_id() {
        let mut request = sample_request();
        request.existing_id = Some("1711678".to_string());

        let server = MockServer::start(
            vec![
                (
                    "GET /developer/api/my/user/getUser",
                    200,
                    r#"{"success":true,"data":{"uccId":"5566"}}"#,
                ),
                (
                    "POST /developer/api/articleDraft/putDraft",
                    200,
                    r#"{"success":true}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "c_csrf=tok123",
            &request,
            PublishOperation::Update,
            &server.base,
        ));

        // 响应不带 aid 时应复用 1711678，而不是报「响应缺少草稿 ID」。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "1711678");
    }

    #[test]
    fn upsert_cookie_appends_when_key_absent() {
        let merged = upsert_cookie("a=1; b=2", "c_csrf", "NEW");
        assert_eq!(merged, "a=1; b=2; c_csrf=NEW");
    }

    #[test]
    fn missing_csrf_is_recovered_from_homepage_set_cookie() {
        // 凭据里只有统一登录域的字段，没有 c_csrf——这正是线上真实情况。
        // 首页下发 Set-Cookie 后应能补全并继续走通整个发布流程。
        let server = MockServer::start_with_headers(
            vec![
                (
                    "GET /developer/api/my/user/getUser",
                    200,
                    r#"{"success":true,"data":{"uccId":"5566"}}"#,
                ),
                (
                    "POST /developer/api/articleDraft/putDraft",
                    200,
                    r#"{"success":true,"data":{"aid":"7007"}}"#,
                ),
                ("GET /", 200, "<html></html>"),
            ],
            (500, "{}"),
            &[("Set-Cookie", "c_csrf=probed456; Path=/; Domain=developer.aliyun.com")],
        );

        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "login_aliyunid=abc; login_current_pkt=xyz",
            &sample_request(),
            PublishOperation::Create,
            &server.base,
        ));

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "7007");
    }

    #[test]
    fn probe_without_csrf_set_cookie_still_fails_as_auth_invalid() {
        // 首页没有下发任何 CSRF 令牌时，不能误判成功，必须仍然提示重新登录。
        let server = MockServer::start_with_headers(
            vec![("GET /", 200, "<html></html>")],
            (500, "{}"),
            &[("Set-Cookie", "session=abc; Path=/")],
        );

        let result = tauri::async_runtime::block_on(publish_with_cookie(
            "login_aliyunid=abc",
            &sample_request(),
            PublishOperation::Create,
            &server.base,
        ));

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::AuthInvalid));
    }
}