use regex::Regex;
use serde_json::{json, Value};

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "tencentcloud_config";
const PLATFORM: &str = "腾讯云开发者社区";
const BASE_URL: &str = "https://cloud.tencent.com/developer";
const TITLE_LIMIT: usize = 80;

pub fn read_cookie(app: &tauri::AppHandle) -> Result<String, (FailureClass, String)> {
    cookie::load_from_store(app, CONFIG_KEY, PLATFORM)
}

/// 腾讯云要求正文用 `<!--markdown-->` 包裹才会按 Markdown 渲染。
fn wrap_markdown(content: &str) -> String {
    format!("<!--markdown-->\n{}\n<!--/markdown-->", content.trim())
}

fn plain_text(content: &str) -> String {
    let mut result = String::new();
    let mut in_fence = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            result.push_str(trimmed);
            result.push('\n');
            continue;
        }

        let mut cleaned = String::new();
        let mut in_tag = false;
        for ch in trimmed.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => cleaned.push(ch),
                _ => {}
            }
        }

        let stripped = cleaned
            .trim_start_matches('#')
            .trim_start_matches(&['>', '*', '-', '+', ' '][..])
            .trim();
        if !stripped.is_empty() {
            result.push_str(stripped);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

fn summary_of(plain: &str) -> String {
    plain.chars().take(200).collect()
}

fn business_error(value: &Value) -> Option<String> {
    if let Some(text) = value
        .get("message")
        .or_else(|| value.get("msg"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn is_success_code(value: &Value) -> bool {
    let status_ok = extract::read_numeric_field(value, &["status"]) == Some(0);
    let code_ok = extract::read_numeric_field(value, &["code"]).unwrap_or(0) == 0;
    status_ok && code_ok
}

/// 判断响应是否在说「这条草稿不存在」。
///
/// `editArticleDraft` 只认 `draftId`，后端查不到就返回 `code: 10002` / `msg: 草稿不存在`。
/// 常见触发场景：用户在腾讯云后台手动删了草稿，而 OpenPub 本地还留着旧 ID。
fn is_draft_missing(body: &str) -> bool {
    body.contains("10002") || body.contains("草稿不存在")
}

/// 发送一次草稿请求并读回 `(status, body)`。
///
/// 抽成独立函数是为了「先编辑、失败再新建」两次请求能共用同一套头部与错误分类，
/// 避免重复写两遍请求构造代码后两边漂移。
async fn send_draft_request(
    client: &reqwest::Client,
    url: &str,
    cookie: &str,
    origin: &str,
    referer: &str,
    payload: &Value,
    operation: PublishOperation,
) -> Result<(u16, String), PublishResult> {
    let pending = client
        .post(url)
        .header("Cookie", cookie)
        .header("Content-Type", "application/json")
        .header("Origin", origin)
        .header("Referer", referer)
        .header("User-Agent", response::USER_AGENT)
        .json(payload)
        .send()
        .await;
    response::read_text(pending, PLATFORM, operation).await
}

/// 从响应原文里兜底捞草稿 ID。
///
/// 腾讯云的 `addArticleDraft` 在草稿确实创建成功后，仍可能返回非 2xx、
/// 返回非 JSON，或给出 `status != 0` 的业务码（网关超时后上游已落库是常见情形）。
/// 此时若直接判定「创建草稿失败」，用户会看到一条失败日志，而网页端其实
/// 已经多出一篇草稿——比报错更糟的是它还会在下次同步时再建一篇。
///
/// 因此这里不信任业务码，改为在响应原文里找 `draftId`：既覆盖 JSON 字段，
/// 也覆盖形如 `...?draftId=343903` 的跳转链接。找到即认为草稿已落库。
fn draft_id_from_body(body: &str) -> Option<String> {
    let regex = Regex::new(r#""draftId"\s*:\s*("?)(\d+)("?)"#).ok()?;
    if let Some(captures) = regex.captures(body) {
        if let Some(id) = captures.get(2) {
            let id = id.as_str().trim();
            if !extract::is_placeholder_id(id) {
                return Some(id.to_string());
            }
        }
    }

    let query_regex = Regex::new(r"draftId=(\d+)").ok()?;
    let captures = query_regex.captures(body)?;
    let id = captures.get(1)?.as_str().trim();
    if extract::is_placeholder_id(id) {
        None
    } else {
        Some(id.to_string())
    }
}

const TAG_SEARCH_PATH: &str = "/api/tag/search";
const TAG_LIMIT: usize = 5;

/// 解析标签搜索响应。响应可能是裸数组，也可能包在 `list` / `data` 里；
/// 每项必须同时给出 `tagId` 与 `tagName` 才可用——发布接口只认数字 `tagIds`。
fn tags_from_response(body: &str) -> Vec<(i64, String)> {
    let value = match response::parse_json(body) {
        Some(value) => value,
        None => return Vec::new(),
    };

    let list = if value.is_array() {
        value.as_array().cloned().unwrap_or_default()
    } else {
        ["list", "data", "tags"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_array).cloned())
            .unwrap_or_default()
    };

    list.iter()
        .filter_map(|item| {
            let id = extract::read_numeric_field(item, &["tagId", "id"])?;
            let name = item
                .get("tagName")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?
                .trim()
                .to_string();
            if id <= 0 || name.is_empty() {
                None
            } else {
                Some((id, name))
            }
        })
        .collect()
}

/// 文本标签解析结果：`ids` 进发布接口的 `tagIds`，其余只能放 `longtailTag`。
struct TagSelection {
    ids: Vec<i64>,
    longtail: Vec<String>,
}

/// 用文本标签查一次标签库，返回同名标签的数字 ID。
///
/// 查询失败时按长尾标签处理而不是直接报错：单个标签查不到不该让整篇发布崩掉，
/// 真正的判据是「最后有没有拿到 tagIds」，那里会给出明确失败。
async fn search_tag(
    client: &reqwest::Client,
    cookie: &str,
    origin: &str,
    referer: &str,
    url: &str,
    keyword: &str,
    operation: PublishOperation,
) -> Option<i64> {
    let pending = client
        .post(url)
        .header("Cookie", cookie)
        .header("Content-Type", "application/json")
        .header("Origin", origin)
        .header("Referer", referer)
        .header("User-Agent", response::USER_AGENT)
        .json(&json!({ "keyword": keyword, "limit": 20 }))
        .send()
        .await;

    let (status, body) = match response::read_text(pending, PLATFORM, operation).await {
        Ok(pair) => pair,
        Err(_) => {
            eprintln!(
                "[tencentcloud] 标签搜索请求失败，标签「{}」按长尾标签处理",
                keyword
            );
            return None;
        }
    };

    if !(200..300).contains(&status) {
        let preview: String = body.chars().take(120).collect();
        eprintln!(
            "[tencentcloud] 标签搜索 HTTP {}，标签「{}」按长尾标签处理: {}",
            status, keyword, preview
        );
        return None;
    }

    let tags = tags_from_response(&body);
    let matched = tags
        .iter()
        .find(|(_, name)| name.to_lowercase() == keyword.to_lowercase())
        .map(|(id, _)| *id);

    eprintln!(
        "[tencentcloud] 标签搜索「{}」命中 {} 条，精确匹配 ID={:?}",
        keyword,
        tags.len(),
        matched
    );

    matched
}

async fn resolve_tags(
    client: &reqwest::Client,
    cookie: &str,
    origin: &str,
    referer: &str,
    base: &str,
    keywords: &[String],
    operation: PublishOperation,
) -> TagSelection {
    let url = format!("{}{}", base, TAG_SEARCH_PATH);
    eprintln!(
        "[tencentcloud] 标签搜索 POST {} keywords={:?}",
        url, keywords
    );
    let mut ids: Vec<i64> = Vec::new();
    let mut longtail: Vec<String> = Vec::new();

    for keyword in keywords {
        if ids.len() >= TAG_LIMIT {
            break;
        }
        match search_tag(client, cookie, origin, referer, &url, keyword, operation).await {
            Some(id) => {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            None => {
                if !longtail.iter().any(|item| item == keyword) {
                    longtail.push(keyword.clone());
                }
            }
        }
    }

    longtail.truncate(TAG_LIMIT.saturating_sub(ids.len()));
    TagSelection { ids, longtail }
}

/// 组装发布请求体。
///
/// 与草稿接口的关键差异：发布必须带 `tagIds`（数字数组），且评论/外链开关叫
/// `banComment` / `closeArticleTextLink`——草稿接口里的 `openComment` /
/// `closeTextLink` 在发布接口上不存在。缺了这些字段服务端只回一句「参数错误」，
/// 因此这里一次列全，避免再退回不可观测的 400。
fn build_publish_payload(
    request: &PublishRequest,
    markdown: &str,
    plain: &str,
    summary: &str,
    draft_id: i64,
    tags: &TagSelection,
) -> Value {
    json!({
        "title": request.title,
        "content": markdown,
        "plain": plain,
        "pic": "",
        "userSummary": "",
        "summary": summary,
        "vlogIds": [],
        "draftId": draft_id,
        "sourceType": 1,
        "classifyIds": [],
        "tagIds": tags.ids,
        "longtailTag": tags.longtail,
        "columnIds": [],
        "banComment": 0,
        "closeArticleTextLink": 0,
    })
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
    if request.title.chars().count() > TITLE_LIMIT {
        return response::business_failure(
            PLATFORM,
            format!("标题不能超过 {} 个字符", TITLE_LIMIT),
            operation,
            false,
        );
    }

    let client = response::new_client();
    let origin = "https://cloud.tencent.com";
    let referer = format!("{}/article/write-new", base);

    let session_url = format!("{}/api/user/session", base);
    let (status, body) = match response::read_text(
        client
            .post(&session_url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/json")
            .header("Origin", origin)
            .header("Referer", &referer)
            .header("User-Agent", response::USER_AGENT)
            .body("{}")
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

    let logged_in = value.get("isLogined").and_then(Value::as_bool) == Some(true)
        && value.get("isQCommunityLogined").and_then(Value::as_bool) == Some(true);
    let user_id = value
        .get("userInfo")
        .and_then(|info| extract::extract_id(info, &["uid"]))
        .unwrap_or_default();
    if !logged_in || user_id.is_empty() {
        return response::auth_failure(
            PLATFORM,
            "未登录，请先登录腾讯云开发者社区",
            operation,
            false,
        );
    }

    let markdown = wrap_markdown(&request.content);
    let plain = plain_text(&request.content);
    let summary = summary_of(&plain);
    let longtail: Vec<String> = request
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .take(5)
        .collect();

   let mut draft_payload = json!({
        "articleId": 0,
        "title": request.title,
        "content": markdown,
        "plain": plain,
        "summary": summary,
        "userSummary": "",
        "sourceType": 0,
        "classifyIds": [],
        "tagIds": [],
        "longtailTag": longtail,
        "columnIds": [],
        "openComment": 1,
        "closeTextLink": 0,
        "pic": "",
        "sourceDetail": {},
        "zoneName": "",
    });

    let mut draft_url = format!("{}/api/article/addArticleDraft", base);
    let mut used_edit_api = false;

    // 更新操作且带旧 ID 时改走编辑草稿接口，避免每次都新建一份草稿。
    // 注意：`articleId` 必须始终保持 0——editArticleDraft 只认 `draftId`，
    // 一旦把草稿 ID 也塞进 articleId，后端会去文章表里查，直接报「草稿不存在」。
    if operation == PublishOperation::Update {
        if let Some(existing_id) = &request.existing_id {
            if let Ok(numeric_id) = existing_id.parse::<i64>() {
                draft_payload["draftId"] = json!(numeric_id);
                draft_url = format!("{}/api/article/editArticleDraft", base);
                used_edit_api = true;
            }
        }
    }

    let (mut status, mut body) = match send_draft_request(
        &client,
        &draft_url,
        cookie,
        origin,
        &referer,
        &draft_payload,
        operation,
    )
    .await
    {
        Ok(pair) => pair,
        Err(failure) => return failure,
    };

    // 云端草稿被手动删除、或换了账号时，拿着旧 ID 走编辑接口只会一直报错。
    // 这里自动降级为新建，避免用户卡在红字报错里出不来。
    let mut fell_back_to_create = false;
    if used_edit_api && is_draft_missing(&body) {
        eprintln!("[tencentcloud] 云端草稿已不存在，自动降级为新建草稿");
        draft_url = format!("{}/api/article/addArticleDraft", base);
        if let Some(obj) = draft_payload.as_object_mut() {
            obj.remove("draftId");
        }
        let (retry_status, retry_body) = match send_draft_request(
            &client,
            &draft_url,
            cookie,
            origin,
            &referer,
            &draft_payload,
            operation,
        )
        .await
        {
            Ok(pair) => pair,
            Err(failure) => return failure,
        };
        status = retry_status;
        body = retry_body;
        fell_back_to_create = true;
    }

    // 先看响应原文里有没有 `draftId`：只要草稿已经落库，无论业务码/状态码
    // 报成什么样，都按成功处理。这是「日志报失败但网页已有草稿」的止血点。
    let recovered_draft_id = draft_id_from_body(&body);

    if !(200..300).contains(&status) {
        match &recovered_draft_id {
            Some(id) => {
                eprintln!(
                    "[tencentcloud] 响应 HTTP {} 但已含 draftId={}，按草稿创建成功处理",
                    status, id
                );
            }
            None => return response::http_failure(PLATFORM, status, &body, operation, false),
        }
    }

    let draft_id = match recovered_draft_id {
        Some(id) => id,
        None => {
            let value = response::parse_json(&body);

            // 腾讯云的 `editArticleDraft` 成功时只回 `{}`（或空 body），既没有 `status`
            // 也没有 `draftId`。若沿用新建接口的严格校验，就会把「网页草稿已覆盖成功」
            // 误判成「拒绝保存草稿」。因此对更新操作放宽：响应里既无 `error`、也无明确
            // 业务码时，按成功处理并复用请求里的旧 ID。
            //
            // 注意不能连带放过 `{"status":-1,...}` 这类真实失败——它带业务码，仍走下面
            // 的严格校验。降级新建后也不适用：那个旧 ID 对应的草稿已被删除。
            let update_succeeded_silently = operation == PublishOperation::Update
                && !fell_back_to_create
                && match value.as_ref() {
                    None => true,
                    Some(v) => {
                        v.get("error").is_none()
                            && v.get("status").is_none()
                            && v.get("code").is_none()
                    }
                };

            if update_succeeded_silently {
                request.existing_id.clone().unwrap_or_else(|| "0".to_string())
            } else {
                let value = match value {
                    Some(value) => value,
                    None => {
                        return response::business_failure(
                            PLATFORM,
                            "保存草稿失败: 响应不是有效 JSON，且未找到草稿 ID",
                            operation,
                            false,
                        )
                    }
                };

                if !is_success_code(&value) || value.get("error").is_some() {
                    let detail = business_error(&value).unwrap_or_else(|| {
                        format!("腾讯云拒绝保存草稿（HTTP {}）: {}", status, body)
                    });
                    return response::business_failure(PLATFORM, detail, operation, false);
                }

                // 更新时响应里不带 draftId 是正常的，直接复用请求里传下来的 ID。
                // 但降级为新建后不能再复用——那个 ID 对应的草稿已被删除。
                if operation == PublishOperation::Update && !fell_back_to_create {
                    request.existing_id.clone().unwrap_or_else(|| "0".to_string())
                } else {
                    match extract::extract_id(&value, &["draftId"]) {
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
            }
        }
    };

    let mut is_published = request.existing_published;
    let mut post_id = draft_id.clone();

    if request.publish_status && !request.existing_published {
        if longtail.is_empty() {
            return response::business_failure(
                PLATFORM,
                "腾讯云发布必须至少填写一个标签",
                operation,
                false,
            );
        }

        let numeric_draft: i64 = match draft_id.parse() {
            Ok(value) => value,
            Err(_) => {
                return response::business_failure(
                    PLATFORM,
                    "草稿 ID 格式异常，无法发布",
                    operation,
                    false,
                )
            }
        };

        // 发布接口只认标签库里的数字 `tagIds`，纯文本标签不算数。
        // 先把用户标签查一遍标签库，拿不到任何 ID 时明确失败——否则发出去必然
        // 换来一句「参数错误」，用户既看不到原因也不知道该改什么。
        let tags = resolve_tags(&client, cookie, origin, &referer, base, &longtail, operation).await;

        if tags.ids.is_empty() {
            return response::business_failure(
                PLATFORM,
                format!(
                    "腾讯云草稿已保存（草稿 ID: {}），但未发布：发布需要至少一个平台标签库中已有的标签，当前标签均未匹配到（{}）。请换成腾讯云已收录的标签后重试",
                    draft_id,
                    longtail.join("、")
                ),
                operation,
                false,
            );
        }

        eprintln!(
            "[tencentcloud] 发布标签 tagIds={:?} longtailTag={:?}",
            tags.ids, tags.longtail
        );

        let publish_payload = build_publish_payload(request, &markdown, &plain, &summary, numeric_draft, &tags);

        let publish_url = format!("{}/api/article/addArticle", base);
        eprintln!(
            "[tencentcloud] 发布请求 POST {} draftId={}",
            publish_url, numeric_draft
        );
        let (status, body) = match response::read_text(
            client
                .post(&publish_url)
                .header("Cookie", cookie)
                .header("Content-Type", "application/json")
                .header("Origin", origin)
                .header("Referer", &referer)
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

        if !is_success_code(&value) || value.get("error").is_some() {
            let detail = business_error(&value)
                .unwrap_or_else(|| "腾讯云拒绝发布请求".to_string());
            return response::business_failure(PLATFORM, detail, operation, false);
        }

        let article_id = extract::extract_id(&value, &["articleId"]).unwrap_or_default();
        if article_id.is_empty() {
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

    // 发布成功后 `draftId` 页面只是草稿编辑页，指向它会让用户以为文章还没上线；
    // 已发布时改给正式文章页，未发布才给草稿编辑页。
    let edit_url = if is_published {
        format!("{}/article/{}", base, post_id)
    } else {
        format!("{}/article/write-new?draftId={}", base, draft_id)
    };
    let message = if is_published {
        format!("已成功发布到腾讯云开发者社区！文章 ID: {}", post_id)
    } else {
        format!("腾讯云草稿已保存！草稿 ID: {}", draft_id)
    };

    PublishResult::success(post_id, edit_url, message, operation, is_published)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::shared::mock::{closed_port_base, MockServer};

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "腾讯云标题".to_string(),
            content: "## 正文\n\n- 条目".to_string(),
            publish_status,
            tags: Some("Rust,云原生".to_string()),
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
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345,"nickname":"张三"}}"#,
                ),
                (
                    "POST /api/article/addArticleDraft",
                    200,
                    r#"{"status":0,"draftId":888}"#,
                ),
                // 发布前必须先把文本标签换成标签库里的数字 ID。
                (
                    "POST /api/tag/search",
                    200,
                    r#"[{"tagId":11,"tagName":"Rust"},{"tagId":12,"tagName":"云原生"}]"#,
                ),
                (
                    "POST /api/article/addArticle",
                    200,
                    r#"{"status":0,"articleId":9009}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "9009");
        assert!(result.is_published);
        // 发布成功要指向正式文章页，而不是草稿编辑页。
        assert_eq!(result.edit_url, format!("{}/article/9009", server.base));
    }

    #[test]
    fn publish_payload_carries_the_fields_add_article_requires() {
        let request = sample_request(true);
        let tags = TagSelection {
            ids: vec![11, 12],
            longtail: Vec::new(),
        };
        let payload = build_publish_payload(&request, "md", "plain", "sum", 888, &tags);

        // 少任何一个都会被服务端统一回「参数错误」，因此逐个钉死。
        assert_eq!(payload["tagIds"], json!([11, 12]));
        assert_eq!(payload["longtailTag"], json!([]));
        assert_eq!(payload["sourceType"], json!(1));
        assert_eq!(payload["banComment"], json!(0));
        assert_eq!(payload["closeArticleTextLink"], json!(0));
        assert_eq!(payload["classifyIds"], json!([]));
        assert_eq!(payload["columnIds"], json!([]));
        assert_eq!(payload["draftId"], json!(888));
        assert_eq!(payload["vlogIds"], json!([]));
        // 草稿接口的开关字段名在发布接口上不存在，不得混用。
        assert!(payload.get("openComment").is_none());
        assert!(payload.get("closeTextLink").is_none());
    }

    #[test]
    fn tags_from_response_accepts_array_and_wrapped_shapes() {
        assert_eq!(
            tags_from_response(r#"[{"tagId":11,"tagName":"Rust"}]"#),
            vec![(11, "Rust".to_string())]
        );
        assert_eq!(
            tags_from_response(r#"{"data":[{"tagId":"12","tagName":"云原生"}]}"#),
            vec![(12, "云原生".to_string())]
        );
        // 缺 ID 或缺名字的条目不可用，宁可丢弃也不猜。
        assert_eq!(
            tags_from_response(r#"[{"tagId":0,"tagName":"x"},{"tagName":"y"}]"#),
            Vec::new()
        );
    }

    #[test]
    fn publish_without_recognized_tag_fails_with_actionable_message() {
        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345,"nickname":"张三"}}"#,
                ),
                (
                    "POST /api/article/addArticleDraft",
                    200,
                    r#"{"status":0,"draftId":888}"#,
                ),
                // 标签库里查不到同名标签：只剩 longtailTag，发布接口必回 400。
                ("POST /api/tag/search", 200, r#"[{"tagId":11,"tagName":"Go"}]"#),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(!result.success, "{}", result.message);
        assert!(!result.is_published);
        // 草稿已经落库，必须如实告知，且说明卡在标签上。
        assert!(result.message.contains("888"), "{}", result.message);
        assert!(result.message.contains("标签"), "{}", result.message);
    }

    #[test]
    fn auth_error_maps_to_auth_invalid() {
        let server = MockServer::start(vec![], (403, "{}"));

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
    fn markdown_is_wrapped_and_plain_is_stripped() {
        let wrapped = wrap_markdown("## 标题\n正文");
        assert!(wrapped.starts_with("<!--markdown-->"));
        assert!(wrapped.ends_with("<!--/markdown-->"));
        assert_eq!(plain_text("## 标题\n正文"), "标题\n正文");
    }

    #[test]
    fn draft_id_is_recovered_from_plain_and_nested_bodies() {
        assert_eq!(
            draft_id_from_body(r#"{"status":0,"draftId":888}"#),
            Some("888".to_string())
        );
        assert_eq!(
            draft_id_from_body(r#"{"draftId":"343903"}"#),
            Some("343903".to_string())
        );
        assert_eq!(
            draft_id_from_body("https://cloud.tencent.com/developer/article/write-new?draftId=343903"),
            Some("343903".to_string())
        );
        assert_eq!(draft_id_from_body(r#"{"draftId":0}"#), None);
        assert_eq!(draft_id_from_body("服务异常"), None);
    }

    #[test]
    fn created_draft_is_reported_success_even_when_code_says_otherwise() {
        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345}}"#,
                ),
                (
                    "POST /api/article/addArticleDraft",
                    500,
                    r#"{"status":-1,"message":"系统繁忙","draftId":343903}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(false), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "343903");
        assert!(result.edit_url.contains("draftId=343903"));
    }

    #[test]
    fn draft_missing_detection_matches_platform_error() {
        assert!(is_draft_missing(r#"{"code":10002,"msg":"草稿不存在"}"#));
        assert!(is_draft_missing(r#"{"code":-1,"msg":"草稿不存在"}"#));
        assert!(!is_draft_missing(r#"{"status":0,"draftId":888}"#));
        assert!(!is_draft_missing(r#"{"code":0,"msg":"成功"}"#));
    }

    #[test]
    fn update_with_existing_id_calls_edit_draft_api() {
        let mut request = sample_request(false);
        request.existing_id = Some("888".to_string());
        request.existing_published = false;

        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345}}"#,
                ),
                (
                    "POST /api/article/editArticleDraft",
                    200,
                    r#"{"status":0}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &request, PublishOperation::Update);

        // 只有真的走了 editArticleDraft 才会返回 200；若误发 addArticleDraft，
        // 未命中路由会落到 500 fallback，这里就会失败。
        assert!(result.success, "{}", result.message);
        assert_eq!(
            result.post_id, "888",
            "更新成功但响应无 draftId 时，应复用请求里的旧 ID"
        );
    }

    #[test]
    fn deleted_draft_falls_back_to_create_and_drops_stale_id() {
        let mut request = sample_request(false);
        request.existing_id = Some("777".to_string());
        request.existing_published = false;

        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345}}"#,
                ),
                (
                    "POST /api/article/editArticleDraft",
                    500,
                    r#"{"code":10002,"msg":"草稿不存在"}"#,
                ),
                (
                    "POST /api/article/addArticleDraft",
                    200,
                    r#"{"status":0,"draftId":999}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &request, PublishOperation::Update);

        // 不降级的话这里会是 http_failure；降级成功则拿到新建返回的 999。
        assert!(result.success, "{}", result.message);
        assert_eq!(
            result.post_id, "999",
            "草稿已删除时必须改用新建返回的新 ID，不能退回失效的 777"
        );
    }

    #[test]
    fn empty_response_on_update_is_treated_as_success() {
        let mut request = sample_request(false);
        request.existing_id = Some("344084".to_string());
        request.existing_published = false;

        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345}}"#,
                ),
                (
                    "POST /api/article/editArticleDraft",
                    200,
                    r#"{}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &request, PublishOperation::Update);

        // editArticleDraft 成功时只回 {}，没有 status/draftId。
        // 必须按成功处理，并把旧 ID 复用回去，否则会误报「拒绝保存草稿」。
        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "344084");
    }

    #[test]
    fn explicit_error_on_update_is_still_a_failure() {
        let mut request = sample_request(false);
        request.existing_id = Some("344084".to_string());
        request.existing_published = false;

        let server = MockServer::start(
            vec![
                (
                    "POST /api/user/session",
                    200,
                    r#"{"isLogined":true,"isQCommunityLogined":true,"userInfo":{"uid":12345}}"#,
                ),
                (
                    "POST /api/article/editArticleDraft",
                    200,
                    r#"{"status":-1,"msg":"内容不能为空"}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &request, PublishOperation::Update);

        // 放宽空对象不代表放过带业务码的真实失败。
        assert!(!result.success, "带 status=-1 的响应必须判失败");
    }
}