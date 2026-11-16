use regex::Regex;
use serde_json::Value;

use super::shared::{cookie, extract, response};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "51cto_config";
const PLATFORM: &str = "51CTO";
const BASE_URL: &str = "https://blog.51cto.com";

/// 51CTO 的后台路径按用途划分，混用会拿到 404 的「文章不存在」页面：
/// - `GET  /blogger/publish`     → 新建文章编辑页（**页面**，不是接口）
/// - `GET  /blogger/draft/{did}` → 已有草稿的编辑页
/// - `POST /blogger/draft`       → 保存/更新草稿，返回 `data.did`
const EDITOR_PATH: &str = "/blogger/publish";
const DRAFT_PATH: &str = "/blogger/draft";

/// 打印真实发出的请求行，便于对照网页端 F12 抓包结果定位路由差异。
fn log_request(method: &str, url: &str) {
    eprintln!("[51cto] 请求 {} {}", method, url);
}

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

/// 脱敏打印 Cookie：只列出键名、不打印值，用于核对是否包含 `_session_id`、
/// `access_token` 等核心鉴权字段。
fn cookie_keys(cookie: &str) -> String {
    let mut names: Vec<&str> = cookie
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| segment.split_once('=').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    names.join(",")
}

/// 响应体片段：去掉换行便于单行打印，`max_chars` 控制长度。
fn body_snippet(body: &str, max_chars: usize) -> String {
    body.trim()
        .chars()
        .take(max_chars)
        .collect::<String>()
        .replace('\n', " ")
        .replace('\r', " ")
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

/// 51CTO 对标签有硬限制：单个标签不超过 20 字符，且服务端按空格二次切分。
/// 因此这里按 `,` `，` `、` 拆分后逐段归一化，再以逗号重新拼接。
const TAG_MAX_CHARS: usize = 20;
const TAG_TOTAL_MAX_CHARS: usize = 100;

/// 返回（提交用的标签串，是否发生精简）。
fn normalize_tags(raw: &str) -> (String, bool) {
    let mut tags: Vec<String> = Vec::new();
    for segment in raw.split(|c| matches!(c, ',' | '，' | '、')) {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tag: String = trimmed
            .chars()
            .take(TAG_MAX_CHARS)
            .map(|c| if c.is_whitespace() { '-' } else { c })
            .collect();
        if tag.is_empty() {
            continue;
        }
        if tags.contains(&tag) {
            continue;
        }
        tags.push(tag.clone());
    }

    let mut kept: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut trimmed_any = false;
    for tag in tags {
        let len = tag.chars().count();
        let next = if kept.is_empty() { len } else { total + 1 + len };
        if next > TAG_TOTAL_MAX_CHARS {
            // 后续标签只会让总长继续变大，整体丢弃并提示，不在标签中间截断
            trimmed_any = true;
            break;
        }
        kept.push(tag);
        total = next;
    }

    (kept.join(","), trimmed_any)
}

/// 发布候选地址。51CTO 没有公开的发布 API 文档，实测结论是：
/// - `POST /blogger/publish` 返回 404「文章不存在」——那是编辑页的 GET 地址，不是接口；
/// - `POST /blogger/draft` 能过，但只是保存草稿，响应不带文章 ID。
///
/// 因此把「带 draft_id 的专用发布路由」排在前面，`check=1` 的保存接口兜底。
/// 只有 404/405 才继续下一个候选：这两个状态说明路由不存在、请求没被业务处理，不会产生
/// 副作用；其余响应立即上报，避免把真实业务错误当成「换个地址再试」。
fn publish_candidates(base: &str, draft_id: &str) -> Vec<String> {
    vec![
        format!("{}{}/{}/publish", base, DRAFT_PATH, draft_id),
        format!("{}{}", base, DRAFT_PATH),
    ]
}

/// 发布响应的文章 ID 字段名各平台不统一，这里把可能的名字一次列全。
const PUBLISH_ID_KEYS: &[&str] = &["blog_id", "article_id", "aid", "id"];

/// 发布响应里的文章地址字段名。
const PUBLISH_URL_KEYS: &[&str] = &["url", "link", "article_url", "blog_url"];

/// 读顶层或 `data` 一层的文本字段。`extract::extract_id` 只处理数字型 ID，
/// 文章地址是字符串，需要单独取值。
fn read_text_field(value: &Value, keys: &[&str]) -> Option<String> {
    fn pick(node: &Value, keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Some(text) = node.get(*key).and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() && !extract::is_placeholder_id(trimmed) {
                    return Some(trimmed.to_string());
                }
            }
        }
        None
    }

    pick(value, keys).or_else(|| value.get("data").and_then(|data| pick(data, keys)))
}

/// 从文章地址末尾取数字段作为文章 ID（`.../u_1234/5678` → `5678`）。
fn id_from_url(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit('/').next()?;
    let digits: String = last.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() >= 3 && !extract::is_placeholder_id(&digits) {
        Some(digits)
    } else {
        None
    }
}

/// 读页面里的 canonical 链接（原样返回，是否算文章页交给调用方判断）。
fn canonical_link(html: &str) -> Option<String> {
    let patterns = [
        r#"<link[^>]*rel=["']canonical["'][^>]*href=["']([^"']+)["']"#,
        r#"<link[^>]*href=["']([^"']+)["'][^>]*rel=["']canonical["']"#,
    ];

    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        if let Some(captures) = regex.captures(html) {
            if let Some(value) = captures.get(1) {
                let url = value.as_str().trim();
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }
    }

    None
}

/// 从详情页里挖出最终文章 ID 与地址。
///
/// **canonical 是唯一可信判据**：草稿编辑页的 canonical 指向草稿自身
/// （`blogger/draft/{did}`），文章页的 canonical 指向文章地址。因此 canonical 落在
/// 草稿路径上时直接判「没发出去」，不看页面里任何 `blog_id`——草稿页会内嵌侧边栏的
/// 历史文章列表，照着那些 ID 认账会得出「已发布」的假结论。
fn parse_article_from_page(html: &str, draft_id: &str) -> Option<(String, String)> {
    if let Some(url) = canonical_link(html) {
        if url.contains("blogger/draft") {
            return None;
        }
        let id = id_from_url(&url)?;
        if id == draft_id {
            return None;
        }
        return Some((id, url));
    }

    // 页面没有 canonical 时才退回内嵌 JSON 的 ID 字段，且必须与 draft_id 不同。
    let patterns = [
        r#""blog_id"\s*:\s*"?([0-9]+)"#,
        r#""article_id"\s*:\s*"?([0-9]+)"#,
    ];

    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        if let Some(captures) = regex.captures(html) {
            if let Some(value) = captures.get(1) {
                let id = value.as_str();
                if !extract::is_placeholder_id(id) && id != draft_id {
                    return Some((id.to_string(), String::new()));
                }
            }
        }
    }

    None
}

/// 发布接口返回成功但没给文章 ID 时，用 `draft_id` 回查一次详情页。
///
/// 回查只用来**补充** ID，不参与成败判定：页面里找不到就返回 `None`，
/// 由调用方如实报「无法确认是否已发布」，绝不因为回查结果改判成功。
async fn lookup_published_article(
    client: &reqwest::Client,
    cookie: &str,
    base: &str,
    draft_id: &str,
    operation: PublishOperation,
) -> Result<Option<(String, String)>, PublishResult> {
    let detail_url = format!("{}{}/{}", base, DRAFT_PATH, draft_id);
    log_request("GET", &detail_url);

    let (status, body, final_url) = match response::read_text_with_headers(
        client
            .get(&detail_url)
            .header("Cookie", cookie)
            .header("User-Agent", response::USER_AGENT)
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("Referer", &detail_url)
            .send()
            .await,
        PLATFORM,
        operation,
    )
    .await
    {
        Ok(response) => (response.status, response.body, response.final_url),
        Err(failure) => return Err(failure),
    };

    eprintln!(
        "[51cto] 发布回查 GET status={} final_url={} body={}",
        status,
        final_url,
        body_snippet(&body, 500)
    );

    if !(200..300).contains(&status) {
        return Ok(None);
    }

    Ok(parse_article_from_page(&body, draft_id).or_else(|| {
        // 详情页被重定向到文章页时，最终 URL 里就带着文章 ID。
        if final_url.contains("/article/") || final_url.contains("/u_") {
            id_from_url(&final_url).map(|id| (id, final_url.clone()))
        } else {
            None
        }
    }))
}

fn article_form(
    request: &PublishRequest,
    category: &Category,
    csrf_token: &str,
    draft_id: &str,
    publishing: bool,
    tags: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("title", request.title.clone()),
        ("content", request.content.clone()),
        ("pid", category.parent_id.clone()),
        ("cate_id", category.child_id.clone()),
        ("custom_id", "0".to_string()),
        ("tag", tags.to_string()),
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

    eprintln!(
        "[51cto] 开始发布 cookie_keys=[{}] cookie_len={}",
        cookie_keys(cookie),
        cookie.len()
    );

    let editor_url = format!("{}{}", base, EDITOR_PATH);
    log_request("GET", &editor_url);
    let (status, body, final_url) = match response::read_text_with_headers(
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
        Ok(response) => (response.status, response.body, response.final_url),
        Err(failure) => return failure,
    };

    eprintln!(
        "[51cto] 编辑页 GET status={} final_url={} body={}",
        status,
        final_url,
        body_snippet(&body, 500)
    );

    if !(200..300).contains(&status) {
        return response::http_failure(PLATFORM, status, &body, operation, false);
    }

    // 未登录时 `blog.51cto.com/blogger/publish` 会被 302 到 `home.51cto.com`
    // 的登录回调页（`?from_service=blog&reback=...login-success...`），状态码
    // 跟随重定向后仍是 200，只查状态码会把「被踢回登录页」当成成功。这里改为
    // 核对最终落地域名：只有仍在博客主站才算真的拿到了创作页。
    let landed_on_editor = final_url.contains("/blogger/")
        && !final_url.contains("home.51cto.com")
        && !final_url.contains("login-success");
    if !landed_on_editor {
        eprintln!(
            "[51cto] 编辑页被重定向到非创作页，判定为未登录 final_url={}",
            final_url
        );
        return response::auth_failure(
            PLATFORM,
            "51CTO 登录态已失效：创作页被重定向到登录页，请重新登录",
            operation,
            false,
        );
    }

    let csrf_token = match extract_csrf(&body) {
        Some(token) => token,
        None => {
            eprintln!(
                "[51cto] 编辑页 CSRF Token 提取失败，响应体前 2000 字符:\n{}",
                body.chars().take(2000).collect::<String>()
            );
            return response::auth_failure(
                PLATFORM,
                "未能从创作页提取 CSRF Token，请重新登录 51CTO",
                operation,
                false,
            )
        }
    };

    let categories_url = format!("{}/category/get-child", base);
    log_request("GET", &categories_url);
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

    eprintln!(
        "[51cto] 分类 GET status={} body={}",
        status,
        body_snippet(&body, 500)
    );

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

    let draft_url = format!("{}{}", base, DRAFT_PATH);
    log_request("POST", &draft_url);
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
    let (tag_field, tags_trimmed) = normalize_tags(request.tags.as_deref().unwrap_or_default());
    let form = article_form(request, &category, &csrf_token, existing_did, false, &tag_field);
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

    eprintln!(
        "[51cto] 草稿 POST status={} body={}",
        status,
        body_snippet(&body, 500)
    );

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
    let mut article_url = String::new();

    if request.publish_status && !request.existing_published {
        let form = article_form(request, &category, &csrf_token, &draft_id, true, &tag_field);
        // 网页端是在草稿编辑页点「发布文章」的，Referer 必须跟着换成草稿页地址。
        let draft_edit_url = format!("{}{}/{}", base, DRAFT_PATH, draft_id);
        let mut published: Option<(String, String)> = None;
        let mut missing_routes: Vec<String> = Vec::new();
        let mut idless_route: Option<String> = None;

        for url in publish_candidates(base, &draft_id) {
            log_request("POST", &url);
            let (status, body) = match response::read_text(
                client
                    .post(&url)
                    .header("Cookie", cookie)
                    .header("Accept", "application/json, text/javascript, */*; q=0.01")
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("User-Agent", response::USER_AGENT)
                    .header("Referer", &draft_edit_url)
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

            // 发布响应是判定成败的唯一依据，JSON 完整打印；HTML 错误页只留片段，
            // 避免把整页「文章不存在」灌进日志。
            let logged = if response::parse_json(&body).is_some() {
                body.clone()
            } else {
                body_snippet(&body, 500)
            };
            eprintln!(
                "[51cto] 发布 POST url={} status={} body={}",
                url, status, logged
            );

            if matches!(status, 404 | 405) {
                eprintln!(
                    "[51cto] 发布候选 {} 返回 {}，判定该路由不存在，换下一个候选",
                    url, status
                );
                missing_routes.push(url);
                continue;
            }

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

            let returned_url = read_text_field(&value, PUBLISH_URL_KEYS).unwrap_or_default();
            if let Some(id) = extract::extract_id(&value, PUBLISH_ID_KEYS) {
                published = Some((id, returned_url));
                break;
            }

            eprintln!(
                "[51cto] 发布接口 {} 返回成功但未带文章 ID，改用 draft_id={} 回查详情",
                url, draft_id
            );
            match lookup_published_article(&client, cookie, base, &draft_id, operation).await {
                Ok(Some(pair)) => {
                    published = Some(pair);
                    break;
                }
                Ok(None) => {
                    // 保存接口带 `check=1` 也能返回 status=1，但那只是存了草稿。
                    // 记下这个候选，全部候选耗尽后如实报「无法确认」。
                    idless_route = Some(url.clone());
                }
                Err(failure) => return failure,
            }
        }

        match published {
            Some((id, url)) => {
                post_id = id;
                article_url = url;
                is_published = true;
            }
            None => {
                // 草稿已经建成，这里必须如实告知「草稿在、但没发出去」，
                // 不能报成功，也不能让用户以为文章丢了。
                let detail = match idless_route {
                    Some(url) => format!(
                        "{} 返回成功但未给出文章 ID，用草稿 ID {} 回查详情也未找到",
                        url, draft_id
                    ),
                    None => format!("{} 均返回 404", missing_routes.join("、")),
                };
                return response::business_failure(
                    PLATFORM,
                    format!(
                        "草稿已保存（ID: {}），但无法确认是否已发布：{}。请在网页端按 F12 抓包确认发布请求",
                        draft_id, detail
                    ),
                    operation,
                    false,
                );
            }
        }
    }

    let edit_url = if article_url.is_empty() {
        format!("{}{}/{}", base, DRAFT_PATH, draft_id)
    } else {
        article_url
    };
    let base_message = if is_published {
        format!("已成功发布到 51CTO！文章 ID: {}", post_id)
    } else {
        format!("51CTO 草稿已保存！草稿 ID: {}", draft_id)
    };
    // 标签被精简时如实告知，避免用户以为标签丢了是程序出错
    let message = if tags_trimmed {
        format!("{}（标签较多，已精简至平台允许的长度）", base_message)
    } else {
        base_message
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
    fn publish_candidates_put_the_draft_id_in_the_route() {
        let candidates = publish_candidates("https://blog.51cto.com", "2115182");
        assert_eq!(
            candidates[0],
            "https://blog.51cto.com/blogger/draft/2115182/publish"
        );
        // 兜底候选是保存接口：实测 `check=1` 能返回 status=1，只是不带文章 ID。
        assert_eq!(candidates[1], "https://blog.51cto.com/blogger/draft");
        // 曾经的错误写法：POST 到编辑页地址 `/blogger/publish`，服务端只会回 404「文章不存在」。
        assert!(
            !candidates.iter().any(|url| url.ends_with("/blogger/publish")),
            "{:?}",
            candidates
        );
    }

    #[test]
    fn success_accepts_alternative_id_and_url_fields() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                // 服务端把文章 ID 叫 `aid`、地址叫 `url`，与 `blog_id` 不同名也要认得出来。
                (
                    "POST /blogger/draft/d55/publish",
                    200,
                    r#"{"status":1,"msg":"success","data":{"aid":"900","url":"https://blog.51cto.com/u_1/900"}}"#,
                ),
                ("POST /blogger/draft", 200, r#"{"status":1,"data":{"did":"d55"}}"#),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "900");
        assert!(result.is_published);
        assert_eq!(result.edit_url, "https://blog.51cto.com/u_1/900");
    }

    #[test]
    fn publish_backfills_the_id_from_the_detail_page_when_response_omits_it() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                ("POST /blogger/draft/d55/publish", 404, "文章不存在"),
                // 保存接口带 check=1 只回 status=1，没有文章 ID，必须靠回查详情页补上。
                ("POST /blogger/draft", 200, r#"{"status":1,"data":{"did":"d55"}}"#),
                (
                    "GET /blogger/draft/d55",
                    200,
                    r#"<html><head><link rel="canonical" href="https://blog.51cto.com/u_1/901"></head><body><script>var cfg = {"blog_id":901};</script></body></html>"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "901");
        assert!(result.is_published);
    }

    #[test]
    fn unconfirmed_publish_is_never_reported_as_success() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                ("POST /blogger/draft/d55/publish", 404, "文章不存在"),
                ("POST /blogger/draft", 200, r#"{"status":1,"data":{"did":"d55"}}"#),
                // 回查也拿不到文章 ID：只能承认「无法确认」，绝不能报成功。
                ("GET /blogger/draft/d55", 200, EDITOR_HTML),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(!result.success, "{}", result.message);
        assert!(!result.is_published);
        assert!(result.message.contains("d55"), "{}", result.message);
        assert!(result.message.contains("无法确认"), "{}", result.message);
    }

    #[test]
    fn id_from_url_takes_the_trailing_number() {
        assert_eq!(
            id_from_url("https://blog.51cto.com/u_1234/5678").as_deref(),
            Some("5678")
        );
        assert_eq!(
            id_from_url("https://blog.51cto.com/u_1234/5678.html").as_deref(),
            Some("5678")
        );
        // 数字段太短或为空时不猜，宁可让上层报「无法确认」。
        assert_eq!(id_from_url("https://blog.51cto.com/u_1234/"), None);
        assert_eq!(id_from_url("https://blog.51cto.com/blogger/draft"), None);
    }

    #[test]
    fn draft_page_with_sidebar_ids_is_not_treated_as_published() {
        // 草稿编辑页会内嵌侧边栏的历史文章列表；canonical 指向草稿本身，
        // 说明页面还在草稿态，页面里的任何 blog_id 都不能当成「已发布」。
        let html = r#"<html><head><link rel="canonical" href="https://blog.51cto.com/blogger/draft/2115182"></head><body><script>var list=[{"blog_id":777}]</script></body></html>"#;
        assert_eq!(parse_article_from_page(html, "2115182"), None);
    }

    #[test]
    fn article_page_canonical_yields_id_and_url() {
        // 属性顺序反过来也要认得出（两种写法都真实存在）。
        let html = r#"<html><head><link href="https://blog.51cto.com/u_1234/5678" rel="canonical"></head></html>"#;
        assert_eq!(
            parse_article_from_page(html, "2115182"),
            Some((
                "5678".to_string(),
                "https://blog.51cto.com/u_1234/5678".to_string()
            ))
        );
    }

    #[test]
    fn publish_falls_back_to_the_save_endpoint_when_dedicated_route_is_missing() {
        let server = MockServer::start(
            vec![
                ("GET /blogger/publish", 200, EDITOR_HTML),
                (
                    "GET /category/get-child",
                    200,
                    r#"{"status":1,"data":[{"id":10,"name":"技术","item":[{"id":11,"name":"后端"}]}]}"#,
                ),
                ("POST /blogger/draft/d55/publish", 404, "文章不存在"),
                // 保存接口同时承担建草稿与「带 check=1 发布」，用同一个 mock 响应覆盖两次调用。
                (
                    "POST /blogger/draft",
                    200,
                    r#"{"status":1,"data":{"did":"d55","blog_id":"902"}}"#,
                ),
            ],
            (500, "{}"),
        );

        let result = run(&server.base, &sample_request(true), PublishOperation::Create);

        assert!(result.success, "{}", result.message);
        assert_eq!(result.post_id, "902");
        assert!(result.is_published);
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

        let update_form = article_form(&request, &category, "csrf", "d77", false, "");
        let did = update_form
            .iter()
            .find(|(key, _)| *key == "did")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(did, "d77");

        let create_form = article_form(&request, &category, "csrf", "", false, "");
        let did = create_form
            .iter()
            .find(|(key, _)| *key == "did")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(did, "");
    }

    #[test]
    fn tags_stay_comma_separated() {
        let (tags, trimmed) = normalize_tags("Rust,后端,前端");
        assert_eq!(tags, "Rust,后端,前端");
        assert!(!trimmed);
    }

    #[test]
    fn tags_split_on_fullwidth_separators() {
        let (tags, trimmed) = normalize_tags("Rust，后端、前端, Rust");
        assert_eq!(tags, "Rust,后端,前端");
        assert!(!trimmed);
    }

    #[test]
    fn long_tag_is_truncated_to_twenty_chars() {
        let raw = "长".repeat(30);
        let (tags, _) = normalize_tags(&raw);
        assert_eq!(tags.chars().count(), TAG_MAX_CHARS, "{}", tags);
    }

    #[test]
    fn inner_space_becomes_hyphen() {
        let (tags, _) = normalize_tags("机器 学习");
        assert_eq!(tags, "机器-学习");
    }

    #[test]
    fn too_many_tags_are_trimmed_with_flag() {
        // 单标签都在 20 字符以内，只有「累加总长」会超 100，正好验证总长兜底。
        let raw = (0..40)
            .map(|i| format!("标签{}", i))
            .collect::<Vec<_>>()
            .join(",");
        let (tags, trimmed) = normalize_tags(&raw);
        assert!(trimmed, "总长超 100 时应标记已精简: {}", tags);
        assert!(tags.chars().count() <= TAG_TOTAL_MAX_CHARS, "{}", tags);
        // 超限时从尾部整体丢弃，前面的标签必须完整保留。
        assert!(tags.starts_with("标签0,"), "{}", tags);
    }

    #[test]
    fn content_field_keeps_markdown_source() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "## 小标题\n\n- a\n- b".to_string(),
            ..Default::default()
        };
        let category = Category {
            parent_id: "10".to_string(),
            child_id: "11".to_string(),
        };
        let form = article_form(&request, &category, "csrf", "", false, "");
        let content = form
            .iter()
            .find(|(key, _)| *key == "content")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(content, request.content, "{}", content);
        assert!(!content.contains("<ul>"), "{}", content);
        assert!(!content.contains("<h2>"), "{}", content);
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