use serde_json::Value;

use super::shared::{cookie, extract, markdown, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "bilibili_config";
const PLATFORM: &str = "哔哩哔哩";
const API_BASE: &str = "https://api.bilibili.com";
const MEMBER_BASE: &str = "https://member.bilibili.com";
/// 原始插件（`docs/index.mjs`）注入的 Referer 就是站点根路径 `member.bilibili.com/`，
/// 不是编辑器页。改成编辑器页反而触发 `-400 请求错误`。
const MEMBER_ROOT: &str = "https://member.bilibili.com/";
const NAV_PATH: &str = "/x/web-interface/nav?build=0&mobi_app=web";
/// 原始插件用 `api.bilibili.com`；B 站迁移到 `member.bilibili.com` 后需一并切换。
const DRAFT_PATH: &str = "/x/article/creative/draft/addupdate";
const DRAFTS_PATH: &str = "/x/article/creative/drafts";
const DRAFT_PAGE_SIZE: &str = "50";
const CSRF_COOKIE: &str = "bili_jct";
const TID_TECH: &str = "4";

/// 原始插件源码（`docs/index.mjs` 混淆前）固定的请求头组合。
///
/// 它通过 Manifest V3 `webRequest` API 只对 `*://api.bilibili.com/*` 注入这两条，
/// 其它头（Cookie / User-Agent / Content-Type）由浏览器自动处理。Rust 侧显式带齐。
fn apply_headers(builder: reqwest::RequestBuilder, cookie: &str) -> reqwest::RequestBuilder {
    builder
        .header("Cookie", cookie)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Origin", MEMBER_BASE)
        .header("Referer", MEMBER_ROOT)
        .header("User-Agent", response::USER_AGENT)
}

/// 把 B 站返回的原始响应打到控制台，用于定位 `-400 请求错误` 这类泛化报错。
///
/// `pnpm tauri dev` 的终端会直接输出，用户复制这一行就能判断是风控、缺参还是登录态问题。
fn log_response(stage: &str, status: u16, body: &str) {
    eprintln!(
        "[bilibili] {} 失败，HTTP {}，响应内容: {}",
        stage,
        status,
        response::snippet(body)
    );
}

/// 逗号分隔的字段名，日志里用来确认「这一版到底发了哪几个参数」。
#[cfg(test)]
fn field_names(fields: &[(String, String)]) -> String {
    fields
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

fn check_operation(request: &PublishRequest) -> Result<(), (FailureClass, String)> {
    if request.title.trim().is_empty() {
        return Err((
            FailureClass::PlatformError,
            "哔哩哔哩专栏标题不能为空，请填写标题后重试".to_string(),
        ));
    }
    if request.content.trim().is_empty() {
        return Err((
            FailureClass::PlatformError,
            "哔哩哔哩专栏正文不能为空，空正文会被编辑器判为无效草稿".to_string(),
        ));
    }
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

fn api_message(value: &Value) -> Option<String> {
    ["message", "msg"]
        .iter()
        .filter_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty() && *text != "0")
        .map(str::to_string)
}

fn auth_error_message(value: &Value) -> String {
    api_message(value).unwrap_or_else(|| "登录态已失效，请重新登录哔哩哔哩".to_string())
}

fn draft_error_message(value: &Value) -> String {
    let code = extract::read_numeric_field(value, &["code"]);
    let hint = match code {
        // `-400` 是 B 站的泛化「请求错误」：多数是缺参数或 Referer/Origin 不被接受，
        // 单独一句「请求错误」没法定位，补上排查方向。
        Some(-400) => "（缺参数或来源校验未通过）",
        Some(-412) => "（触发风控，请降低同步频率）",
        Some(-101) | Some(-111) => "（登录态或 csrf 失效，请重新登录）",
        _ => "",
    };

    if let Some(text) = api_message(value) {
        return match code {
            Some(code) => format!("{}{}（code: {}）", text, hint, code),
            None => format!("{}{}", text, hint),
        };
    }
    match code {
        Some(code) => format!("草稿保存失败，接口返回 code: {}{}", code, hint),
        None => "草稿保存失败：响应缺少错误信息".to_string(),
    }
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
            format!("{} {}", PLATFORM, auth_error_message(&value)),
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

/// 草稿更新锚点。B 站专栏编辑页 URL 是 `?aid=...`，`addupdate` 接口也靠 `aid`
/// 判断「改哪条草稿」：`aid` 为 `0` 时新建，为有效 ID 时更新。
///
/// 注意 `pgc_id` 是**付费专栏/课程**的 ID，普通专栏恒为 `0`。早期版本误把旧 aid
/// 写进 `pgc_id`，接口直接忽略，于是每次同步都新建一条草稿（ID 从 373728 一路
/// 递增到 373729、373730），旧草稿随即被 B 站回收，打开编辑页就提示「该草稿已失效」。
fn draft_aid(request: &PublishRequest, operation: PublishOperation) -> String {
    if operation != PublishOperation::Update {
        return "0".to_string();
    }
    request
        .existing_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !extract::is_placeholder_id(id))
        .unwrap_or("0")
        .to_string()
}

fn build_fields(
    request: &PublishRequest,
    content: &str,
    csrf: &str,
    operation: PublishOperation,
) -> Vec<(String, String)> {
    // 与原始插件（docs/index.mjs）完全一致的纯净字段集：
    // {tid:"4", title:e.title, content:s, csrf:this.csrf, save:"0", pgc_id:"0"}
    //
    // 不加多余字段：B 站对冗余字段敏感，多余的（list_id/template_id/image_urls/
    // banner_url/original 等）在正确账号环境下反而可能被风控误伤。
    // 更新时额外带 aid，新建时不带（aid=0 会被判为非法参数）。
    let mut fields = vec![
        ("tid".to_string(), TID_TECH.to_string()),
        ("title".to_string(), request.title.clone()),
        ("content".to_string(), content.to_string()),
        ("csrf".to_string(), csrf.to_string()),
        ("save".to_string(), "0".to_string()),
        ("pgc_id".to_string(), "0".to_string()),
    ];
    let aid = draft_aid(request, operation);
    if aid != "0" {
        fields.push(("aid".to_string(), aid));
    }
    fields
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

    publish_with_cookie(&cookie, request, operation, API_BASE, API_BASE).await
}

async fn publish_with_cookie(
    cookie: &str,
    request: &PublishRequest,
    operation: PublishOperation,
    nav_base: &str,
    draft_base: &str,
) -> PublishResult {
    let csrf = match cookie::require_value(cookie, CSRF_COOKIE, PLATFORM) {
        Ok(value) => value,
        Err(message) => {
            return PublishResult::failure(FailureClass::NotConfigured, message, operation, false)
        }
    };

    let client = response::new_client();

    let nav_url = format!("{}{}", nav_base, NAV_PATH);
    let (status, body) = match response::read_text(
        apply_headers(client.get(&nav_url), cookie).send().await,
        PLATFORM,
        operation,
    )
    .await
    {
        Ok(pair) => pair,
        Err(failure) => return failure,
    };

    if !(200..300).contains(&status) {
        log_response("登录状态预检", status, &body);
        return response::http_failure(PLATFORM, status, &body, operation, false);
    }

    if let Err((class, message)) = parse_nav(&body) {
        return PublishResult::failure(class, message, operation, false);
    }

    // 草稿列表只作诊断，失败不阻断发布；接口已迁到 member.bilibili.com。
    let existing_id = request
        .existing_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !extract::is_placeholder_id(id));
    if let (Some(aids), Some(existing)) = (
        list_draft_aids(&client, draft_base, cookie, &csrf, operation).await,
        existing_id,
    ) {
        let alive = aids.iter().any(|aid| aid == existing);
        eprintln!(
            "[bilibili] 草稿列表共 {} 条，existing_id={} 是否命中: {}",
            aids.len(),
            existing,
            alive
        );
    }

    let content = markdown::to_html_content(&request.content);
    let fields = build_fields(request, &content, &csrf, operation);

    let (value, rebuilt) = match save_draft(&client, draft_base, cookie, &fields, operation).await {
        Ok(value) => (value, false),
        Err(failure) => {
            // 旧草稿被 B 站回收（早期版本每次同步都新建，旧 aid 随之失效）时，
            // 更新必然报「该草稿已失效」。此时降级为新建一次，让同步映射重新拿到
            // 有效 aid，而不是让用户永久卡在一条死 ID 上反复失败。
            if operation != PublishOperation::Update || !is_stale_draft(&failure.message) {
                return failure;
            }
            let retry_fields = build_fields(request, &content, &csrf, PublishOperation::Create);
            match save_draft(&client, draft_base, cookie, &retry_fields, PublishOperation::Create)
                .await
            {
                Ok(value) => (value, true),
                Err(failure) => return failure,
            }
        }
    };

    let aid = match value
        .get("data")
        .and_then(|data| extract::extract_id(data, &["aid", "id"]))
    {
        Some(id) => id,
        None => {
            // 覆盖已有草稿时接口可能只回 code:0 而不带 aid，复用提交的旧 aid。
            // 但降级新建后旧 ID 已失效，不能再回退到它。
            let fallback = if !rebuilt && operation == PublishOperation::Update {
                request
                    .existing_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !extract::is_placeholder_id(id))
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
    let message = if rebuilt {
        format!("哔哩哔哩原草稿已失效，已新建草稿。草稿 ID: {}", aid)
    } else {
        format!("哔哩哔哩草稿已保存！草稿 ID: {}", aid)
    };

    PublishResult::success(aid, edit_url, message, operation, false)
}

/// 旧草稿已被删除/回收时 B 站返回的业务文案，命中才允许降级新建。
fn is_stale_draft(message: &str) -> bool {
    ["失效", "不存在", "已删除", "无效"]
        .iter()
        .any(|keyword| message.contains(keyword))
}

/// 拉取草稿箱列表里的 `aid`，用于诊断「旧草稿是否已被 B 站回收」。
///
/// 返回 `None` 表示这次拿不到（风控、接口变更、网络问题都算），调用方不得因此失败。
/// 成功时返回第一页（最多 `DRAFT_PAGE_SIZE` 条）的 aid 列表。
async fn list_draft_aids(
    client: &reqwest::Client,
    api_base: &str,
    cookie: &str,
    csrf: &str,
    operation: PublishOperation,
) -> Option<Vec<String>> {
    let url = format!("{}{}", api_base, DRAFTS_PATH);
    let query = [
        ("pn", "1"),
        ("ps", DRAFT_PAGE_SIZE),
        ("tid", TID_TECH),
        ("csrf", csrf),
    ];

    let (status, body) = match response::read_text(
        apply_headers(client.get(&url).query(&query), cookie).send().await,
        PLATFORM,
        operation,
    )
    .await
    {
        Ok(pair) => pair,
        Err(_) => return None,
    };

    if !(200..300).contains(&status) {
        log_response("获取草稿列表", status, &body);
        return None;
    }

    let value = match response::parse_json(&body) {
        Some(value) => value,
        None => {
            log_response("获取草稿列表", status, &body);
            return None;
        }
    };

    if extract::read_numeric_field(&value, &["code"]) != Some(0) {
        log_response("获取草稿列表", status, &body);
        return None;
    }

    let data = value.get("data")?;
    // 列表在不同版本里分别挂在 `drafts` / `list` / `data` 本身，按顺序探测，
    // 探测不到就当这次没拿到，不影响发布主流程。
    let items = data
        .get("drafts")
        .and_then(Value::as_array)
        .or_else(|| data.get("list").and_then(Value::as_array))
        .or_else(|| data.as_array())?;

    let aids: Vec<String> = items
        .iter()
        .filter_map(|item| extract::extract_id(item, &["aid", "id"]))
        .collect();
    Some(aids)
}

async fn save_draft(
    client: &reqwest::Client,
    api_base: &str,
    cookie: &str,
    fields: &[(String, String)],
    operation: PublishOperation,
) -> Result<Value, PublishResult> {
    let draft_url = format!("{}{}", api_base, DRAFT_PATH);

    let (status, body) = response::read_text(
        // 不要手动设置 Content-Type：`.form()` 自己会设，多设一次在 reqwest 里
        // 会追加成两个同名头，服务端可能因此判为格式错误。
        apply_headers(client.post(&draft_url).form(fields), cookie)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await?;

    if !(200..300).contains(&status) {
        log_response("保存草稿", status, &body);
        return Err(response::http_failure(
            PLATFORM, status, &body, operation, false,
        ));
    }

    let value = response::parse_json(&body).ok_or_else(|| {
        log_response("保存草稿", status, &body);
        response::business_failure(
            PLATFORM,
            format!("草稿保存失败: 响应不是有效 JSON: {}", response::snippet(&body)),
            operation,
            false,
        )
    })?;

    if extract::read_numeric_field(&value, &["code"]) != Some(0) {
        log_response("保存草稿", status, &body);
        return Err(response::business_failure(
            PLATFORM,
            draft_error_message(&value),
            operation,
            false,
        ));
    }

    Ok(value)
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
        };

        // 与原始插件 docs/index.mjs 完全一致的纯净字段集
        assert_eq!(value("tid"), Some("4"));
        assert_eq!(value("title"), Some("B站专栏标题"));
        assert_eq!(value("content"), Some("<p>第一段正文</p>"));
        assert_eq!(value("csrf"), Some("csrf-token"));
        assert_eq!(value("save"), Some("0"));
        assert_eq!(value("pgc_id"), Some("0"));
        assert_eq!(value("aid"), None, "新建时不应发送 aid");

        // 冗余字段已回退，B 站对多余字段敏感，正确账号环境下也不应携带
        for name in [
            "list_id",
            "template_id",
            "is_manuscript",
            "image_urls",
            "banner_url",
            "original",
        ] {
            assert!(value(name).is_none(), "不应再发送 {}", name);
        }
    }

    #[test]
    fn empty_content_is_rejected_before_request() {
        let request = PublishRequest {
            title: "B站专栏标题".to_string(),
            content: "   \n\n ".to_string(),
            ..Default::default()
        };
        let err = check_operation(&request).unwrap_err();
        assert_eq!(err.0, FailureClass::PlatformError);
        assert!(err.1.contains("正文不能为空"), "{}", err.1);
    }

    #[test]
    fn empty_title_is_rejected_before_request() {
        let request = PublishRequest {
            title: "  ".to_string(),
            content: "第一段正文".to_string(),
            ..Default::default()
        };
        let err = check_operation(&request).unwrap_err();
        assert_eq!(err.0, FailureClass::PlatformError);
        assert!(err.1.contains("标题不能为空"), "{}", err.1);
    }

    /// 锁住 `Vec<(String, String)>` + `.form()` 的线上格式。
    ///
    /// 曾怀疑它被序列化成 `0=tid&1=4`，实测是标准 `key=value`：`serde_urlencoded`
    /// 对二元组序列本身就是按键值对编码，换成 `HashMap` 只会让键序随机。
    #[test]
    fn form_body_encodes_as_key_value_pairs() {
        let fields = build_fields(
            &sample_request(),
            "<p>第一段正文</p>",
            "csrf-token",
            PublishOperation::Create,
        );

        let request = reqwest::Client::new()
            .post("http://127.0.0.1/form-probe")
            .form(&fields)
            .build()
            .expect("build form request");
        let body = request.body().and_then(reqwest::Body::as_bytes).unwrap_or_default();
        let text = String::from_utf8_lossy(body).to_string();

        assert!(text.contains("tid=4"), "{}", text);
        assert!(text.contains("csrf=csrf-token"), "{}", text);
        assert!(text.contains("pgc_id=0"), "{}", text);
        assert!(!text.contains("0=tid"), "{}", text);
        assert!(!text.contains("1=4"), "{}", text);
    }

    /// `Content-Type` 只能出现一次：`.form()` 自己会设置，手动再设一遍会追加同名头。
    #[test]
    fn form_request_has_single_content_type_header() {
        let fields = build_fields(
            &sample_request(),
            "<p>x</p>",
            "csrf-token",
            PublishOperation::Create,
        );

        let request = apply_headers(
            reqwest::Client::new()
                .post("http://127.0.0.1/form-probe")
                .form(&fields),
            "SESSDATA=abc; bili_jct=csrf-token",
        )
        .build()
        .expect("build form request");

        let count = request.headers().get_all("content-type").iter().count();
        assert_eq!(count, 1, "Content-Type 头重复会让 B 站直接回 -400");

        let referer = request
            .headers()
            .get("referer")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert_eq!(referer, MEMBER_ROOT);

        let accept = request
            .headers()
            .get("accept")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert_eq!(accept, "application/json, text/plain, */*");
    }

    #[test]
    fn draft_fields_stay_at_core_set() {
        let request = PublishRequest {
            tags: Some("技术, 开源".to_string()),
            ..sample_request()
        };
        let fields = build_fields(
            &request,
            "<p>第一段正文</p>",
            "csrf-token",
            PublishOperation::Create,
        );
        let value = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap_or("")
        };

        // 与原始插件 docs/index.mjs 完全一致：Create 只有 6 个字段
        assert_eq!(field_names(&fields), "tid,title,content,csrf,save,pgc_id");
        // 冗余字段与旧猜测字段一律不得携带
        for name in [
            "summary",
            "aid",
            "list_id",
            "template_id",
            "is_manuscript",
            "image_urls",
            "banner_url",
            "original",
        ] {
            assert!(value(name).is_empty(), "不应再发送 {}", name);
        }
    }

    #[test]
    fn request_error_message_carries_actionable_hint() {
        let value: Value = serde_json::from_str(r#"{"code":-400,"message":"请求错误"}"#).unwrap();
        let message = draft_error_message(&value);
        assert!(message.contains("请求错误"), "{}", message);
        assert!(message.contains("-400"), "{}", message);
        assert!(message.contains("缺参数"), "{}", message);
    }

    #[test]
    fn draft_list_failure_does_not_block_publish() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "GET /x/article/creative/drafts",
                    200,
                    r#"{"code":-412,"message":"请求被拦截"}"#,
                ),
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
    }

    #[test]
    fn draft_list_extracts_aids_from_nested_lists() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "GET /x/article/creative/drafts",
                    200,
                    r#"{"code":0,"message":"0","data":{"drafts":[{"aid":9527001},{"aid":9527002}]}}"#,
                ),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":0,"message":"0","data":{"aid":9527001}}"#,
                ),
            ],
            (500, "{}"),
        );

        let request = PublishRequest {
            existing_id: Some("9527001".to_string()),
            ..sample_request()
        };
        let result = run(&server.base, &request, PublishOperation::Update);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9527001");
    }

    #[test]
    fn update_uses_existing_id_as_aid() {
        let request = PublishRequest {
            existing_id: Some("9527001".to_string()),
            ..sample_request()
        };
        let fields = build_fields(&request, "<p>x</p>", "csrf-token", PublishOperation::Update);
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };
        assert_eq!(field("aid"), "9527001");
        assert_eq!(field("pgc_id"), "0");
    }

    #[test]
    fn update_falls_back_to_existing_id_when_response_has_no_aid() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":0,"message":"0","data":{}}"#,
                ),
            ],
            (500, "{}"),
        );

        let request = PublishRequest {
            existing_id: Some("373728".to_string()),
            ..sample_request()
        };
        let result = run(&server.base, &request, PublishOperation::Update);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "373728");
        assert!(result.edit_url.contains("aid=373728"));
    }

    #[test]
    fn create_without_aid_in_response_is_failure() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":0,"message":"0","data":{}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
    }

    #[test]
    fn business_error_surfaces_platform_message() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":-412,"message":"该草稿已失效"}"#,
                ),
            ],
            (500, "{}"),
        );

        let request = PublishRequest {
            existing_id: Some("373728".to_string()),
            ..sample_request()
        };
        let result = run(&server.base, &request, PublishOperation::Update);

        assert!(!result.success);
        assert_eq!(result.failure_class, Some(FailureClass::PlatformError));
        assert!(result.message.contains("该草稿已失效"), "{}", result.message);
    }

    #[test]
    fn business_error_without_message_reports_code() {
        let server = MockServer::start(
            vec![
                ("GET /x/web-interface/nav", 200, NAV_OK),
                (
                    "POST /x/article/creative/draft/addupdate",
                    200,
                    r#"{"code":-412,"message":"0"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(), PublishOperation::Create);

        assert!(!result.success);
        assert!(result.message.contains("-412"), "{}", result.message);
    }

    #[test]
    fn stale_draft_detection_only_matches_real_dead_ids() {
        assert!(is_stale_draft("该草稿已失效"));
        assert!(is_stale_draft("稿件不存在"));
        assert!(!is_stale_draft("请求过于频繁"));
        assert!(!is_stale_draft("csrf 校验失败"));
    }

    #[test]
    fn parses_nav_and_rejects_logged_out_payload() {
        assert_eq!(parse_nav(NAV_OK), Ok("10086".to_string()));

        let logged_out = parse_nav(r#"{"code":0,"data":{"isLogin":false}}"#).unwrap_err();
        assert_eq!(logged_out.0, FailureClass::AuthInvalid);
    }
}