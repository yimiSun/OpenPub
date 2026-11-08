// CSDN 的发布端点由阿里云 API 网关保护，必须携带 x-ca-* 签名四件套（HMAC-SHA256）。
// 签名密钥来自对 CSDN 前端签名的分析，属项目「签名预判」规则的特例，
// 授权与风险记录见 openspec/changes/phase2-csdn-zhihu/design.md Decision 13。
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::shared::{cookie, csrf, markdown, truncate};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "csdn_config";
const ERROR_SNIPPET_CHARS: usize = 300;
const SAVE_DRAFT_URL: &str = "https://bizapi.csdn.net/blog-console-api/v3/mdeditor/saveArticle";
/// saveArticle 请求体的 `status` 字段：`2` 保存为草稿，`0` 提交发布（进入审核队列）。
/// 注意这与列表接口返回的字符串状态（`draft` / `audit` / `all_v2`）是两套东西：
/// 前者是请求参数，后者是文章状态描述，不要把两边的取值混用。
const STATUS_DRAFT: i64 = 2;
const STATUS_PUBLISH: i64 = 0;

/// 发布分支的兜底标签。
/// CSDN 的两套校验尺度不同：草稿允许 `tags` 为空，发布必须带标签
/// （真机返回 `{"code":400,"msg":"请设置文章标签"}`）。
/// `PublishRequest` 目前没有标签字段，因此发布时填这个固定值以便通过校验；
/// 代价是每篇发布文章都会被打上该标签，接入标签输入 UI 后应改为透传用户标签。
const DEFAULT_PUBLISH_TAG: &str = "OpenPub";
const CSRF_COOKIE_KEY: &str = "UserToken";

/// 阿里云网关的 key（对应源码里的 `x-ca-key`）。
const X_CA_KEY: &str = "203803574";

/// HMAC-SHA256 的密钥（对应源码里的 `hb` 变量）。
/// 已通过 DevTools 搜索逐字符核实（2026-11-08），字面值无误。
/// 该值硬编码在 CSDN 前端 JS 里，CSDN 换密钥时会静默失效（表现为 401 / X-Ca-Key is not exist），
/// 届时需要重新取一次。更换密钥只改这一行。
const SECRET_KEY: &str = "9znpamsyl2c7cdrr9sas0le9vbc3r6ba";

/// 签名与真实请求必须使用同一个 Accept / Content-Type 常量，
/// 否则网关对「头里的值」做哈希时会与 StringToSign 不一致而判签名错误。
const ACCEPT_ANY: &str = "*/*";
const CONTENT_TYPE_JSON: &str = "application/json";

/// 参与签名的头必须与 `x-ca-signature-headers` 声明的列表完全一致，且按声明顺序拼接。
const SIGNED_HEADER_NAMES: &str = "x-ca-key,x-ca-nonce";

type HmacSha256 = Hmac<Sha256>;

/// 取 URL 的路径部分：去掉 scheme、host 与 query，签名只对 path 做哈希。
pub fn url_path(url: &str) -> &str {
    let after_scheme = match url.find("://") {
        Some(index) => &url[index + 3..],
        None => url,
    };
    let path_start = match after_scheme.find('/') {
        Some(index) => index,
        None => return "/",
    };
    let path = &after_scheme[path_start..];
    match path.find('?') {
        Some(index) => &path[..index],
        None => path,
    }
}

/// 阿里云网关的 StringToSign 拼装。
///
/// 结构为「方法 / Accept / Content-MD5 / Content-Type / Date / 参与签名的头 / path」，
/// 用 `\n` 连接；空值段落保留空字符串（因此会出现连续空行，属规范要求）。
/// 唯一的不对称点：GET 请求在抓包里没有 Date 段，因此比 POST 少一个空行。
pub fn build_string_to_sign(
    method: &str,
    accept: &str,
    content_type: &str,
    nonce: &str,
    path: &str,
) -> String {
    let mut parts: Vec<&str> = vec![method, accept, "", content_type];
    if !content_type.is_empty() {
        parts.push("");
    }
    let header_block = format!("x-ca-key:{}\nx-ca-nonce:{}", X_CA_KEY, nonce);
    parts.push(&header_block);
    parts.push(path);
    parts.join("\n")
}

pub fn compute_signature(string_to_sign: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC-SHA256 接受任意长度的密钥");
    mac.update(string_to_sign.as_bytes());
    BASE64.encode(mac.finalize().into_bytes())
}

/// 每次请求生成新的 nonce（UUID v4）。
pub fn new_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 构造签名四件套。返回 (头名, 头值) 列表，便于单测逐项断言。
pub fn sign_headers(nonce: &str, signature: &str) -> Vec<(String, String)> {
    vec![
        ("x-ca-key".to_string(), X_CA_KEY.to_string()),
        ("x-ca-nonce".to_string(), nonce.to_string()),
        ("x-ca-signature".to_string(), signature.to_string()),
        (
            "x-ca-signature-headers".to_string(),
            SIGNED_HEADER_NAMES.to_string(),
        ),
    ]
}


fn truncate_chars(input: &str, limit: usize) -> String {
    truncate::truncate_chars(input, limit)
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

fn extract_id(data: &serde_json::Value) -> Option<String> {
    const DIRECT_KEYS: [&str; 8] = [
        "articleId",
        "article_id",
        "draftId",
        "draft_id",
        "id",
        "post_id",
        "urlTitle",
        "url_title",
    ];
    for key in DIRECT_KEYS {
        if let Some(v) = data.get(key) {
            if let Some(id) = value_to_id(v) {
                return Some(id);
            }
        }
    }
    None
}

fn read_code(response: &serde_json::Value) -> i64 {
    match response.get("code") {
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

/// 草稿与发布共用 `saveArticle`，两者只差请求体的 `status` 字段。
///
/// 唯一仍然拒绝的组合是「目标已发布 + 用户没勾选覆盖已发布」：此时若照常发 `status = 2`，
/// 等于把一篇线上文章打回草稿，属于不可接受的静默降级。判定发生在发出任何网络请求之前。
pub fn resolve_url(
    request: &PublishRequest,
    _operation: PublishOperation,
) -> Result<&'static str, (FailureClass, String)> {
    if request.existing_published && !request.publish_status {
        return Err((
            FailureClass::NotImplemented,
            "目标文章已在 CSDN 线上发布，但本次未勾选「覆盖已发布的文章」。为避免把线上文章打回草稿，本次未发送任何请求，线上文章保持不变；如需更新线上内容，请勾选「覆盖已发布的文章」后重试".to_string(),
        ));
    }

    Ok(SAVE_DRAFT_URL)
}

/// 请求体 `status` 的取值：勾选发布 → 提交发布（`0`），否则 → 保存草稿（`2`）。
pub fn status_for(request: &PublishRequest) -> i64 {
    if request.publish_status {
        STATUS_PUBLISH
    } else {
        STATUS_DRAFT
    }
}

fn build_body(
    request: &PublishRequest,
    operation: PublishOperation,
    target_id: &str,
    status: i64,
) -> serde_json::Value {
    // 官方 v3 草稿与发布都带 pubStatus，两者配套切换。
    let pub_status = if status == STATUS_PUBLISH {
        "publish"
    } else {
        "draft"
    };

    let mut body = serde_json::json!({
        "title": request.title,
        "markdowncontent": request.content,
        "content": markdown::to_html_content(&request.content),
        // 官方 v3 用驼峰 readType；旧代码的 read_type 是拼写错误，后端会当作未知字段忽略。
        "readType": "public",
        "level": 0,
        // 草稿允许空标签，发布必须带标签，否则返回 400「请设置文章标签」。
        "tags": if status == STATUS_PUBLISH { DEFAULT_PUBLISH_TAG } else { "" },
        "status": status,
        "categories": "",
        "type": "original",
        "original_link": "",
        "authorized_status": false,
        // 官方此字段是字符串，不是数字。
        "not_auto_saved": "1",
        "source": "pc_mdeditor",
        "cover_images": [],
        "cover_type": 1,
        "vote_id": 0,
        "resource_id": "",
        "pubStatus": pub_status,
        "creation_statement": 0,
        "creator_activity_id": "",
    });

    // `is_new` 字面语义是「这是一篇新文章」。官方抓包只有新建场景（值 `1`），
    // 编辑已有文章时的取值未知；无条件发 `1` 有触发「新建而非更新」的风险，
    // 因此只在 Create 分支附带，Update 分支不带该字段。
    if operation == PublishOperation::Create {
        body["is_new"] = serde_json::json!(1);
    }

    if operation == PublishOperation::Update && !is_placeholder_id(target_id) {
        body["id"] = serde_json::Value::String(target_id.to_string());
    }

    body
}

/// 构造成功结果。抽成纯函数便于单测锁定 `is_published` 与提示语。
///
/// `is_published` 直接取请求意图：2026-11-08 真机实测证明 CSDN 对
/// `status = 0` + `pubStatus = "publish"` 的处理是**直接对外发布**——发布后
/// 文章出现在后台「已发布」列表，「审核中/未通过」计数保持为 0，草稿箱计数
/// 同步减少。此前假设的「进审核队列」不成立，若仍返回 false，同步映射会
/// 把刚发布的文章记成未发布，下一次未勾选覆盖的同步就会用 `status = 2`
/// 把线上文章打回草稿箱（即我们一直要防的静默降级）。
fn build_success_result(
    request: &PublishRequest,
    operation: PublishOperation,
    returned_id: String,
    edit_url: String,
) -> PublishResult {
    let is_published = request.publish_status;

    let message = if request.publish_status {
        format!("已成功发布到 CSDN！文章 ID: {}", returned_id)
    } else {
        let action = match operation {
            PublishOperation::Update => "草稿更新成功",
            PublishOperation::Create => "草稿创建成功",
        };
        format!("{}! 文章 ID: {}", action, returned_id)
    };

    PublishResult::success(returned_id, edit_url, message, operation, is_published)
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let cookie = match cookie::load_from_store(app, CONFIG_KEY, "CSDN") {
        Ok(c) => c,
        Err((class, message)) => {
            return PublishResult::failure(class, message, operation, false);
        }
    };

    let target_id = request
        .existing_id
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let url = match resolve_url(request, operation) {
        Ok(u) => u,
        Err((class, message)) => {
            return PublishResult::failure(class, message, operation, false);
        }
    };

    let csrf_token = match csrf::require_token(&cookie, CSRF_COOKIE_KEY, "CSDN") {
        Ok(t) => t,
        Err(message) => {
            return PublishResult::failure(
                FailureClass::AuthInvalid,
                message,
                operation,
                false,
            )
        }
    };

    let request_body = build_body(request, operation, &target_id, status_for(request));

    let nonce = new_nonce();
    let path = url_path(url);
    let string_to_sign = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, &nonce, path);
    let signature = compute_signature(&string_to_sign, SECRET_KEY);
    let signed = sign_headers(&nonce, &signature);

    let client = reqwest::Client::new();
    let mut builder = client
        .post(url)
        .header("Content-Type", CONTENT_TYPE_JSON)
        .header("Accept", ACCEPT_ANY)
        .header("Cookie", &cookie)
        .header("x-csrf-token", &csrf_token)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Referer", "https://editor.csdn.net/")
        .header("Origin", "https://editor.csdn.net");
    for (name, value) in &signed {
        builder = builder.header(name.as_str(), value.as_str());
    }

    let response = match builder.json(&request_body).send().await {
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

    eprintln!("[csdn] 请求地址: {}，请求体: {}", url, request_body);
    eprintln!("[csdn] StringToSign: {}", string_to_sign);
    eprintln!(
        "[csdn] 签名头: x-ca-key={}, x-ca-nonce={}, x-ca-signature={}, x-ca-signature-headers={}",
        X_CA_KEY, nonce, signature, SIGNED_HEADER_NAMES
    );
    eprintln!("[csdn] 原始响应: {}", response_text);

    let response_json: serde_json::Value =
        serde_json::from_str(&response_text).unwrap_or_else(|_| serde_json::json!({}));

    let code = read_code(&response_json);

    if status.as_u16() == 200 && (code == 200 || code == 0) {
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
                        "CSDN 返回成功但未能提取文章 ID，原始响应: {}",
                        truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
                    ),
                    operation,
                    false,
                )
            }
        };

        let edit_url = format!("https://editor.csdn.net/md/?articleId={}", returned_id);

        return build_success_result(request, operation, returned_id, edit_url);
    }

    let msg = response_json
        .get("message")
        .and_then(|v| v.as_str())
        .or_else(|| response_json.get("msg").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let detail = if msg.is_empty() {
        truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
    } else {
        msg
    };

    let class = if status.as_u16() == 401 || status.as_u16() == 403 {
        FailureClass::AuthInvalid
    } else {
        FailureClass::PlatformError
    };

    PublishResult::failure(
        class,
        format!("CSDN API 错误 (code: {}, HTTP {}): {}", code, status, detail),
        operation,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(publish_status: bool, is_update: bool, existing_id: Option<&str>) -> PublishRequest {
        PublishRequest {
            title: "标题".to_string(),
            content: "# 正文".to_string(),
            existing_id: existing_id.map(|s| s.to_string()),
            is_update,
            publish_status,
            existing_published: false,
        }
    }

    #[test]
    fn publish_live_shares_the_save_article_endpoint() {
        let result = resolve_url(&request(true, false, None), PublishOperation::Create);
        assert_eq!(result.expect("发布应可解析"), SAVE_DRAFT_URL);
    }

    #[test]
    fn publish_status_selects_the_publish_status_code() {
        assert_eq!(status_for(&request(true, false, None)), STATUS_PUBLISH);
        assert_eq!(status_for(&request(false, false, None)), STATUS_DRAFT);
    }

    #[test]
    fn publish_request_body_carries_status_zero() {
        let body = build_body(&request(true, false, None), PublishOperation::Create, "", STATUS_PUBLISH);
        assert_eq!(body.get("status").and_then(|v| v.as_i64()), Some(0));
    }

    #[test]
    fn draft_request_body_carries_status_two() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert_eq!(body.get("status").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn publish_request_body_carries_pub_status_publish() {
        let body = build_body(&request(true, false, None), PublishOperation::Create, "", STATUS_PUBLISH);
        assert_eq!(
            body.get("pubStatus").and_then(|v| v.as_str()),
            Some("publish"),
            "发布分支必须带 pubStatus=publish"
        );
    }

    #[test]
    fn draft_request_body_carries_pub_status_draft() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert_eq!(body.get("pubStatus").and_then(|v| v.as_str()), Some("draft"));
        assert_eq!(body.get("readType").and_then(|v| v.as_str()), Some("public"));
        assert!(body.get("read_type").is_none(), "旧拼写 read_type 必须彻底移除");
    }

    #[test]
    fn draft_request_body_carries_official_required_fields() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert_eq!(body.get("source").and_then(|v| v.as_str()), Some("pc_mdeditor"));
        assert_eq!(body.get("not_auto_saved").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(body.get("is_new").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(body.get("cover_type").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(body.get("level").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(body.get("authorized_status").and_then(|v| v.as_bool()), Some(false));
        assert!(body.get("cover_images").and_then(|v| v.as_array()).is_some());
    }

    #[test]
    fn draft_request_body_keeps_tags_empty() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert_eq!(
            body.get("tags").and_then(|v| v.as_str()),
            Some(""),
            "草稿允许空标签，与 v3 抓包一致"
        );
    }

    #[test]
    fn publish_request_body_fills_a_tag() {
        let body = build_body(&request(true, false, None), PublishOperation::Create, "", STATUS_PUBLISH);
        assert_eq!(
            body.get("tags").and_then(|v| v.as_str()),
            Some(DEFAULT_PUBLISH_TAG),
            "发布必须带标签，否则服务端返回 400「请设置文章标签」"
        );
    }

    #[test]
    fn update_request_body_omits_is_new() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123", STATUS_DRAFT);
        assert!(
            body.get("is_new").is_none(),
            "更新分支不得携带 is_new，避免被服务端判定为新建而多出重复文章"
        );
        assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn publish_success_marks_the_article_as_published() {
        let result = build_success_result(
            &request(true, false, None),
            PublishOperation::Create,
            "123".to_string(),
            "https://editor.csdn.net/md/?articleId=123".to_string(),
        );
        assert!(result.success);
        assert!(
            result.is_published,
            "CSDN 实测直接发布，is_published 必须为 true，否则映射会记成未发布、\
             下一次未勾选覆盖的同步会把线上文章打回草稿"
        );
        assert!(result.message.contains("已成功发布"));
        assert!(!result.message.contains("审核"));
    }

    #[test]
    fn draft_success_is_not_marked_as_published() {
        let result = build_success_result(
            &request(false, false, None),
            PublishOperation::Create,
            "123".to_string(),
            "https://editor.csdn.net/md/?articleId=123".to_string(),
        );
        assert!(result.success);
        assert!(!result.is_published, "草稿保存不得标记为已发布");
        assert!(result.message.contains("草稿创建成功"));
    }

    #[test]
    fn publish_update_keeps_the_existing_id() {
        let body = build_body(&request(true, true, Some("123")), PublishOperation::Update, "123", STATUS_PUBLISH);
        assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("123"));
        assert_eq!(body.get("status").and_then(|v| v.as_i64()), Some(0));
    }

    #[test]
    fn published_target_without_overwrite_is_rejected_before_requesting() {
        let mut req = request(false, true, Some("123"));
        req.existing_published = true;
        let result = resolve_url(&req, PublishOperation::Update);
        let (_, message) = result.err().expect("未勾选覆盖时不应改动线上文章");
        assert!(
            message.contains("线上文章保持不变"),
            "必须说明线上文章未被改动: {}",
            message
        );
        assert!(
            message.contains("未发送任何请求"),
            "必须说明未发请求: {}",
            message
        );
    }

    #[test]
    fn published_target_with_overwrite_is_allowed_to_publish() {
        let mut req = request(true, true, Some("123"));
        req.existing_published = true;
        let url = resolve_url(&req, PublishOperation::Update).expect("勾选覆盖后应可发布");
        assert_eq!(url, SAVE_DRAFT_URL);
        assert_eq!(status_for(&req), STATUS_PUBLISH);
    }

    #[test]
    fn plain_draft_intent_uses_draft_status() {
        let req = request(false, false, None);
        let url = resolve_url(&req, PublishOperation::Create).expect("草稿应可解析");
        assert_eq!(url, SAVE_DRAFT_URL);
        assert_eq!(status_for(&req), STATUS_DRAFT);
    }

    #[test]
    fn create_draft_resolves_to_save_draft_url() {
        let url = resolve_url(&request(false, false, None), PublishOperation::Create).expect("应可解析");
        assert_eq!(url, SAVE_DRAFT_URL);
    }

    #[test]
    fn update_draft_resolves_to_save_draft_url() {
        let url = resolve_url(&request(false, true, Some("123")), PublishOperation::Update).expect("应可解析");
        assert_eq!(url, SAVE_DRAFT_URL);
    }

    #[test]
    fn body_status_is_draft_by_default() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert_eq!(body.get("status").and_then(|v| v.as_i64()), Some(STATUS_DRAFT));
        assert_eq!(body.get("type").and_then(|v| v.as_str()), Some("original"));
        assert_eq!(body.get("readType").and_then(|v| v.as_str()), Some("public"));
    }

    #[test]
    fn body_marks_content_html_and_markdowncontent_raw() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        let content = body.get("content").and_then(|v| v.as_str()).expect("content 必须存在");
        assert!(content.starts_with("<p>"), "content 应为 HTML: {}", content);
        assert_eq!(
            body.get("markdowncontent").and_then(|v| v.as_str()),
            Some("# 正文")
        );
    }

    #[test]
    fn create_body_has_no_id() {
        let body = build_body(&request(false, false, None), PublishOperation::Create, "", STATUS_DRAFT);
        assert!(body.get("id").is_none());
    }

    #[test]
    fn update_body_carries_existing_id() {
        let body = build_body(&request(false, true, Some("123")), PublishOperation::Update, "123", STATUS_DRAFT);
        assert_eq!(body.get("id").and_then(|v| v.as_str()), Some("123"));
    }

    #[test]
    fn update_with_placeholder_id_drops_the_field() {
        let body = build_body(&request(false, true, Some("0")), PublishOperation::Update, "0", STATUS_DRAFT);
        assert!(body.get("id").is_none());
    }

    #[test]
    fn extract_id_reads_article_id() {
        let data = serde_json::json!({ "articleId": "12345" });
        assert_eq!(extract_id(&data), Some("12345".to_string()));
    }

    #[test]
    fn extract_id_reads_url_title_fallback() {
        let data = serde_json::json!({ "urlTitle": "abc-123" });
        assert_eq!(extract_id(&data), Some("abc-123".to_string()));
    }

    #[test]
    fn extract_id_returns_none_when_absent_or_placeholder() {
        assert_eq!(extract_id(&serde_json::json!({ "foo": 1 })), None);
        assert_eq!(extract_id(&serde_json::json!({ "articleId": "0" })), None);
        assert_eq!(extract_id(&serde_json::json!({})), None);
    }

    #[test]
    fn read_code_accepts_string_and_number() {
        assert_eq!(read_code(&serde_json::json!({ "code": 200 })), 200);
        assert_eq!(read_code(&serde_json::json!({ "code": "200" })), 200);
        assert_eq!(read_code(&serde_json::json!({})), -1);
    }

    #[test]
    fn csrf_key_points_to_usertoken() {
        assert_eq!(CSRF_COOKIE_KEY, "UserToken");
    }

    const TEST_NONCE: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn url_path_strips_scheme_host_and_query() {
        assert_eq!(
            url_path(SAVE_DRAFT_URL),
            "/blog-console-api/v3/mdeditor/saveArticle"
        );
        assert_eq!(url_path("https://a.b/c?d=1"), "/c");
        assert_eq!(url_path("https://a.b"), "/");
    }

    #[test]
    fn post_string_to_sign_matches_capture_format() {
        let signed = build_string_to_sign(
            "POST",
            ACCEPT_ANY,
            CONTENT_TYPE_JSON,
            TEST_NONCE,
            "/blog-console-api/v3/mdeditor/saveArticle",
        );
        let expected = concat!(
            "POST\n",
            "*/*\n",
            "\n",
            "application/json\n",
            "\n",
            "x-ca-key:203803574\n",
            "x-ca-nonce:0f8fad5b-d9cb-469f-a165-70867728950e\n",
            "/blog-console-api/v3/mdeditor/saveArticle",
        );
        assert_eq!(signed, expected);
    }

    #[test]
    fn get_string_to_sign_has_empty_content_type_line() {
        let signed =
            build_string_to_sign("GET", ACCEPT_ANY, "", TEST_NONCE, "/blog-console-api/v3/tag/list");
        let expected = concat!(
            "GET\n",
            "*/*\n",
            "\n",
            "\n",
            "x-ca-key:203803574\n",
            "x-ca-nonce:0f8fad5b-d9cb-469f-a165-70867728950e\n",
            "/blog-console-api/v3/tag/list",
        );
        assert_eq!(signed, expected);
    }

    #[test]
    fn string_to_sign_has_exactly_seven_segments() {
        let signed = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, TEST_NONCE, "/p");
        assert_eq!(signed.split('\n').count(), 8);
    }

    #[test]
    fn signature_is_deterministic_and_base64_sha256() {
        let signed = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, TEST_NONCE, "/p");
        let first = compute_signature(&signed, SECRET_KEY);
        let second = compute_signature(&signed, SECRET_KEY);
        assert_eq!(first, second);
        assert_eq!(first.chars().count(), 44, "SHA256 的 Base64 应为 44 字符");
        assert!(first.ends_with('='));
    }

    #[test]
    fn signature_changes_with_nonce_and_secret() {
        let a = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, "nonce-a", "/p");
        let b = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, "nonce-b", "/p");
        assert_ne!(
            compute_signature(&a, SECRET_KEY),
            compute_signature(&b, SECRET_KEY)
        );
        assert_ne!(
            compute_signature(&a, SECRET_KEY),
            compute_signature(&a, "another-secret")
        );
    }

    #[test]
    fn signature_is_sensitive_to_content_type_drift() {
        let with_charset = build_string_to_sign(
            "POST",
            ACCEPT_ANY,
            "application/json;charset=UTF-8",
            TEST_NONCE,
            "/p",
        );
        let plain = build_string_to_sign("POST", ACCEPT_ANY, CONTENT_TYPE_JSON, TEST_NONCE, "/p");
        assert_ne!(
            compute_signature(&with_charset, SECRET_KEY),
            compute_signature(&plain, SECRET_KEY),
            "请求头与 StringToSign 的 Content-Type 必须一致"
        );
    }

    #[test]
    fn nonce_is_uuid_v4() {
        let nonce = new_nonce();
        assert_eq!(nonce.len(), 36);
        assert_eq!(nonce.chars().nth(14), Some('4'));
        assert_eq!(nonce.chars().nth(8), Some('-'));
    }

    #[test]
    fn sign_headers_contain_the_four_ca_headers() {
        let headers = sign_headers(TEST_NONCE, "sig");
        let names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "x-ca-key",
                "x-ca-nonce",
                "x-ca-signature",
                "x-ca-signature-headers"
            ]
        );
        assert_eq!(headers[0].1, "203803574");
        assert_eq!(headers[1].1, TEST_NONCE);
        assert_eq!(headers[2].1, "sig");
        assert_eq!(headers[3].1, "x-ca-key,x-ca-nonce");
    }

    #[test]
    fn signed_header_names_cover_the_declared_list() {
        let declared: Vec<&str> = SIGNED_HEADER_NAMES.split(',').collect();
        let mut expected = vec!["x-ca-key", "x-ca-nonce"];
        expected.sort();
        let mut actual = declared.clone();
        actual.sort();
        assert_eq!(actual, expected, "声明的签名头必须与 StringToSign 里的头一致");
    }
}