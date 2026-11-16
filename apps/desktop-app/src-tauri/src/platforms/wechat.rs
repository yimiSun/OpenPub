use super::shared::{cookie, markdown, truncate};
use super::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const CONFIG_KEY: &str = "wechat_config";

const ERROR_SNIPPET_CHARS: usize = 300;
const DIGEST_LIMIT: usize = 120;
const IMAGE_PLACEHOLDER: &str = "[图片]";
const API_BASE: &str = "https://mp.weixin.qq.com/cgi-bin/operate_appmsg";
const EDITOR_BASE: &str = "https://mp.weixin.qq.com/cgi-bin/appmsg";
const HOME_URL: &str = "https://mp.weixin.qq.com/";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// create 时 AppMsgId 必须是空串。参考实现（docs/index.mjs 的 `saveDraft`）在
/// `sub=create` 分支里传的是 `AppMsgId:""`；传 `"0"` 会被服务端当成一次对
/// AppMsgId=0 的更新，直接返回 `ret=200002 参数错误`。
const CREATE_APPMSG_ID: &str = "";

/// 抓包里固定出现在 content0 末尾的尾巴，不是装饰。
/// `data-value=3` 推测为「纯文本模式」，阶段 A 照抄。
const CONTENT_TAIL: &str =
    r#"<p style="display: none;"><mp-style-type data-value="3"></mp-style-type></p>"#;

/// 字段清单严格对齐 docs/index.mjs 中微信 `saveDraft` 的请求对象。
///
/// 这里**不包含 `fingerprint`**：参考实现从未发送该字段，微信侧也不要求它。
/// 之前用 32 个 0 占位反而被服务端判成非法指纹，正是 `ret=200002 参数错误`
/// 的直接来源。`articlenum` / `req` / `save_type` / `isneedsave` /
/// `is_auto_type_saving` / `remind_flag` 同样不在参考实现里，一并移除。
const STATIC_FIELDS: &[(&str, &str)] = &[
    ("lang", "zh_CN"),
    ("f", "json"),
    ("ajax", "1"),
    ("operate_from", "Chrome"),
    ("isnew", "0"),
    // 单篇固定为 1；参考实现没有 articlenum，也没有任何 1 系列索引字段
    ("count", "1"),
    ("data_seq", "0"),
    ("ad_video_transition0", ""),
    ("can_reward0", "0"),
    ("related_video0", ""),
    ("is_video_recommend0", "-1"),
    ("writerid0", "0"),
    ("fileid0", ""),
    // content_source_url 允许为空，但字段本身必须出现
    ("sourceurl0", ""),
    ("need_open_comment0", "1"),
    ("only_fans_can_comment0", "0"),
    ("cdn_url0", ""),
    ("cdn_235_1_url0", ""),
    ("cdn_16_9_url0", ""),
    ("cdn_3_4_url0", ""),
    ("cdn_1_1_url0", ""),
    ("cdn_url_back0", ""),
    ("crop_list0", ""),
    ("last_choose_cover_from0", "0"),
    ("app_cover_auto0", "0"),
    ("multi_picture_cover0", "0"),
    ("music_id0", ""),
    ("video_id0", ""),
    ("voteid0", ""),
    ("voteismlt0", ""),
    ("supervoteid0", ""),
    ("cardid0", ""),
    ("cardquantity0", ""),
    ("cardlimit0", ""),
    ("vid_type0", ""),
    ("show_cover_pic0", "0"),
    ("shortvideofileid0", ""),
    ("copyright_type0", "0"),
    ("releasefirst0", ""),
    ("platform0", ""),
    ("reprint_permit_type0", ""),
    ("allow_reprint0", ""),
    ("allow_reprint_modify0", ""),
    ("original_article_type0", ""),
    ("ori_white_list0", ""),
    ("free_content0", ""),
    ("fee0", "0"),
    ("ad_id0", ""),
    ("guide_words0", ""),
    ("is_share_copyright0", "0"),
    ("share_copyright_url0", ""),
    ("source_article_type0", ""),
    ("reprint_recommend_title0", ""),
    ("reprint_recommend_content0", ""),
    ("share_page_type0", "0"),
    ("share_imageinfo0", r#"{"list":[]}"#),
    ("share_video_id0", ""),
    ("dot0", "{}"),
    ("share_voice_id0", ""),
    ("insert_ad_mode0", ""),
    ("categories_list0", "[]"),
];

pub struct Credentials {
    pub cookie: String,
    pub token: String,
}

pub fn read_credentials(app: &tauri::AppHandle) -> Result<Credentials, (FailureClass, String)> {
    let cookie = cookie::load_from_store(app, CONFIG_KEY, "微信公众号")?;

    let token = cookie::load_config_field(app, CONFIG_KEY, "token")
        .map_err(|e| (FailureClass::NotConfigured, e))?
        .ok_or_else(|| {
            (
                FailureClass::NotConfigured,
                "未填写 token：请在公众号后台地址栏里复制 token 参数的值后填入配置".to_string(),
            )
        })?;

    Ok(Credentials { cookie, token })
}

pub fn check_operation(
    request: &PublishRequest,
    _operation: PublishOperation,
) -> Result<(), (FailureClass, String)> {
    if request.existing_published {
        return Err((
            FailureClass::NotImplemented,
            "微信公众号文章发布后无法修改正文，已发布文章的同步更新暂不支持".to_string(),
        ));
    }
    if request.publish_status {
        return Err((
            FailureClass::NotImplemented,
            "微信公众号仅支持保存草稿，请到公众号后台手动发布".to_string(),
        ));
    }
    Ok(())
}

pub fn build_url(token: &str, operation: PublishOperation) -> String {
    let sub = match operation {
        PublishOperation::Create => "create",
        PublishOperation::Update => "update",
    };
    format!(
        "{}?t=ajax-response&sub={}&type=77&token={}&lang=zh_CN",
        API_BASE, sub, token
    )
}

fn find_matching(chars: &[char], start: usize, open: char, close: char) -> Option<usize> {
    if chars.get(start) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    for (offset, ch) in chars.iter().enumerate().skip(start) {
        if *ch == open {
            depth += 1;
        } else if *ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

/// 把 Markdown 图片语法 `![alt](url)` 替换为指定文本。
/// 先剥图片再转 HTML，否则草稿正文会残留 `![](...)` 垃圾。
fn strip_images(input: &str, replacement: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some(bracket_end) = find_matching(&chars, i + 1, '[', ']') {
                if chars.get(bracket_end + 1) == Some(&'(') {
                    if let Some(paren_end) = find_matching(&chars, bracket_end + 1, '(', ')') {
                        out.push_str(replacement);
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 把普通链接 `[text](url)` 收敛成 `text`，用于摘要纯文本化。
fn strip_link_targets(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(bracket_end) = find_matching(&chars, i, '[', ']') {
                if chars.get(bracket_end + 1) == Some(&'(') {
                    if let Some(paren_end) = find_matching(&chars, bracket_end + 1, '(', ')') {
                        let text: String = chars[i + 1..bracket_end].iter().collect();
                        out.push_str(&text);
                        i = paren_end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 微信编辑器只接受受控的 HTML 子集。这里在转换后的 HTML 上摘掉整段
/// `<script>` / `<style>` / `<iframe>` / `<object>` / `<embed>`，
/// 避免它们被编辑器当成可执行内容（`comrak` 开启了原始 HTML 透传，
/// 用户手写的标签会原样出现在产物里）。
fn strip_unsupported_tags(input: &str) -> String {
    let mut text = input.to_string();
    for tag in ["script", "style", "iframe", "object", "embed"] {
        let open = format!("<{}", tag);
        let close = format!("</{}", tag);
        loop {
            // `to_ascii_lowercase` 不改变字节长度，可以安全地拿它做偏移量查找
            let lower = text.to_ascii_lowercase();
            let Some(start) = lower.find(&open) else {
                break;
            };
            let Some(open_end) = lower[start..].find('>').map(|rel| start + rel) else {
                text.truncate(start);
                break;
            };
            let end = match lower[open_end..].find(&close) {
                Some(rel) => {
                    let after = open_end + rel;
                    lower[after..]
                        .find('>')
                        .map(|gt| after + gt + 1)
                        .unwrap_or(text.len())
                }
                None => text.len(),
            };
            text.replace_range(start..end, "");
        }
    }
    text
}

/// 找到 `<tag` 后紧跟空白、`>` 或 `/` 的起始位置，避免把 `<article>`
/// 之类的前缀误判成 `<a>`。
fn find_tag_start(lower: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{}", tag);
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&needle) {
        let start = from + rel;
        let next = lower[start + needle.len()..].chars().next();
        if matches!(next, Some(' ') | Some('>') | Some('/') | Some('\n') | Some('\t')) {
            return Some(start);
        }
        from = start + needle.len();
    }
    None
}

/// 把 HTML 里的 `<img ...>` 整体替换成 `[图片]`：公众号正文不允许外链图片，
/// 直接透传会在保存草稿时被拒收。
fn replace_img_tags(input: &str) -> String {
    let mut text = input.to_string();
    loop {
        let lower = text.to_ascii_lowercase();
        let Some(start) = find_tag_start(&lower, "img") else {
            break;
        };
        let Some(rel_end) = lower[start..].find('>') else {
            text.truncate(start);
            break;
        };
        let end = start + rel_end + 1;
        text.replace_range(start..end, IMAGE_PLACEHOLDER);
    }
    text
}

/// `<a ...>文字</a>` 收敛为 `文字`：公众号正文不支持可点外链，
/// 保留 `<a>` 会在草稿里留下点了没反应的死链。
fn collapse_anchors(input: &str) -> String {
    let mut text = input.to_string();
    loop {
        let lower = text.to_ascii_lowercase();
        let Some(start) = find_tag_start(&lower, "a") else {
            break;
        };
        let Some(open_rel) = lower[start..].find('>') else {
            text.truncate(start);
            break;
        };
        let open_end = start + open_rel + 1;
        match lower[open_end..].find("</a") {
            Some(rel) => {
                let close_start = open_end + rel;
                let close_end = lower[close_start..]
                    .find('>')
                    .map(|gt| close_start + gt + 1)
                    .unwrap_or(text.len());
                let inner = text[open_end..close_start].to_string();
                text.replace_range(start..close_end, &inner);
            }
            None => text.replace_range(start..open_end, ""),
        }
    }
    text
}

fn sanitize_html(input: &str) -> String {
    let without_blocks = strip_unsupported_tags(input);
    let without_images = replace_img_tags(&without_blocks);
    collapse_anchors(&without_images)
}

/// content0：先把 Markdown 转成 HTML，再在 HTML 上做微信侧清洗
/// （摘危险标签、图片换占位文本、链接收敛为文字），
/// 末尾拼上抓包里固定出现的 `mp-style-type` 尾巴。
pub fn build_content(markdown_source: &str) -> String {
    let body = sanitize_html(&markdown::to_html_content(markdown_source));
    format!("{}{}", body, CONTENT_TAIL)
}

/// 摘要用纯文本：剥图片、链接收敛成文字、去标题井号与强调符号、合并空白。
pub fn plain_text(input: &str) -> String {
    let without_images = strip_images(input, "");
    let without_links = strip_link_targets(&without_images);
    let mut parts: Vec<String> = Vec::new();
    for line in without_links.lines() {
        let stripped = line.trim().trim_start_matches('#').trim();
        let cleaned: String = stripped
            .chars()
            .filter(|c| !matches!(c, '`' | '*'))
            .collect();
        let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            parts.push(collapsed);
        }
    }
    parts.join(" ")
}

fn clip_chars(input: &str, limit: usize) -> String {
    if input.chars().count() <= limit {
        return input.to_string();
    }
    let mut clipped: String = input.chars().take(limit.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

pub fn build_digest(markdown_source: &str, limit: usize) -> String {
    clip_chars(&plain_text(markdown_source), limit)
}

/// 参考实现里 `random` 是 `String(Math.random())`，即一个 0~1 的小数串。
/// 用纳秒低位拼一个同形状的值即可，微信只要求它不是常量。
fn random_field_value() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    format!("0.{}", nanos)
}

pub fn build_fields(
    request: &PublishRequest,
    operation: PublishOperation,
    target_id: &str,
    token: &str,
) -> Vec<(String, String)> {
    let appmsg_id = match operation {
        PublishOperation::Create => CREATE_APPMSG_ID,
        PublishOperation::Update => target_id,
    };

    // 微信摘要上限 120 字；为空时交给 `auto_gen_digest0` 自动生成。
    let digest = build_digest(&request.content, DIGEST_LIMIT);
    let auto_gen_digest = if digest.is_empty() { "1" } else { "0" };

    let mut fields: Vec<(String, String)> = vec![
        // token / lang 在 URL 里也有，但参考实现的 body 里同样带，两边都传
        ("token".to_string(), token.to_string()),
        ("random".to_string(), random_field_value()),
        ("title0".to_string(), request.title.clone()),
        // PublishRequest 没有作者字段，留空由公众号后台默认值兜底
        ("author0".to_string(), String::new()),
        ("digest0".to_string(), digest),
        ("auto_gen_digest0".to_string(), auto_gen_digest.to_string()),
        ("content0".to_string(), build_content(&request.content)),
    ];

    for (name, value) in STATIC_FIELDS {
        fields.push((name.to_string(), value.to_string()));
    }

    fields.push(("AppMsgId".to_string(), appmsg_id.to_string()));

    fields
}

fn build_form(fields: &[(String, String)]) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new();
    for (name, value) in fields {
        form = form.text(name.clone(), value.clone());
    }
    form
}

fn describe_fields(fields: &[(String, String)]) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}(len={})", name, value.chars().count()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 排查 `200002 参数错误` 时需要看到实际发送的正文，
/// 只打长度无法判断格式问题。默认截前 100 个字符。
fn preview_fields(fields: &[(String, String)], names: &[&str], limit: usize) -> String {
    names
        .iter()
        .filter_map(|name| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| format!("{}={}", name, truncate::truncate_chars(value, limit)))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_ret(value: &serde_json::Value) -> Option<i64> {
    for key in ["ret", "errcode"] {
        if let Some(raw) = value.get(key) {
            if let Some(n) = raw.as_i64() {
                return Some(n);
            }
            if let Some(s) = raw.as_str() {
                if let Ok(n) = s.trim().parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }

    if let Some(base) = value.get("base_resp") {
        return read_ret(base);
    }

    None
}

/// ret 与 errmsg 必须取自同一个对象：`base_resp` 结构下两者都在嵌套层里。
fn ret_holder(value: &serde_json::Value) -> &serde_json::Value {
    for key in ["ret", "errcode"] {
        if value.get(key).is_some() {
            return value;
        }
    }
    if let Some(base) = value.get("base_resp") {
        if base.get("ret").is_some() || base.get("errcode").is_some() {
            return base;
        }
    }
    value
}

pub struct ParsedResponse {
    pub success: bool,
    pub ret: Option<i64>,
    pub detail: String,
    pub json: Option<serde_json::Value>,
}

/// 微信大量错误同样返回 HTTP 200，因此成功判定只看响应体。
/// 解析不出 JSON 或缺少 ret 一律判失败——不能把网关错误页当成成功。
pub fn parse_response(body: &str) -> ParsedResponse {
    let trimmed = body.trim();

    let json = match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) if value.is_object() => value,
        _ => {
            return ParsedResponse {
                success: false,
                ret: None,
                detail: format!(
                    "响应不是 JSON 对象: {}",
                    truncate::truncate_chars(trimmed, ERROR_SNIPPET_CHARS)
                ),
                json: None,
            };
        }
    };

    let ret = read_ret(&json);
    let detail = ret_holder(&json)
        .get("errmsg")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match ret {
        Some(0) => ParsedResponse {
            success: true,
            ret,
            detail: if detail.is_empty() {
                "ok".to_string()
            } else {
                detail
            },
            json: Some(json),
        },
        Some(n) => ParsedResponse {
            success: false,
            ret,
            detail: if detail.is_empty() {
                format!("ret={} 且响应体无 errmsg", n)
            } else {
                detail
            },
            json: Some(json),
        },
        None => ParsedResponse {
            success: false,
            ret: None,
            detail: format!(
                "响应缺少 ret 字段: {}",
                truncate::truncate_chars(trimmed, ERROR_SNIPPET_CHARS)
            ),
            json: Some(json),
        },
    }
}

fn is_placeholder_id(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    trimmed.is_empty() || trimmed == "0" || trimmed == "未知" || trimmed == "(未知)"
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

pub fn extract_appmsg_id(value: &serde_json::Value) -> Option<String> {
    const DIRECT_KEYS: [&str; 6] = [
        "AppMsgId",
        "appMsgId",
        "appmsgid",
        "app_msg_id",
        "appmsg_id",
        "id",
    ];
    for key in DIRECT_KEYS {
        if let Some(raw) = value.get(key) {
            if let Some(id) = value_to_id(raw) {
                return Some(id);
            }
        }
    }

    for key in ["data", "appMsg", "appmsg"] {
        if let Some(nested) = value.get(key) {
            if let Some(id) = extract_appmsg_id(nested) {
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

fn build_edit_url(appmsg_id: &str, token: &str) -> String {
    if appmsg_id.is_empty() {
        return HOME_URL.to_string();
    }
    format!(
        "{}?t=media/appmsg_edit&action=edit&type=77&appmsgid={}&token={}&lang=zh_CN",
        EDITOR_BASE, appmsg_id, token
    )
}

pub async fn publish(
    app: &tauri::AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    if let Err((class, message)) = check_operation(request, operation) {
        return PublishResult::failure(class, message, operation, false);
    }

    let target_id = read_target_id(request);
    if operation == PublishOperation::Update && target_id.is_empty() {
        return PublishResult::failure(
            FailureClass::NotConfigured,
            "更新草稿需要已有的 AppMsgId，但当前映射记录缺少该 ID".to_string(),
            operation,
            false,
        );
    }

    let credentials = match read_credentials(app) {
        Ok(c) => c,
        Err((class, message)) => {
            return PublishResult::failure(class, message, operation, false);
        }
    };

    let url = build_url(&credentials.token, operation);
    let fields = build_fields(request, operation, &target_id, &credentials.token);
    let form = build_form(&fields);

    eprintln!("[wechat] 请求地址: {}", url);
    eprintln!("[wechat] 请求字段: {}", describe_fields(&fields));
    eprintln!(
        "[wechat] 关键字段值: token={}, AppMsgId={}",
        credentials.token,
        fields
            .iter()
            .find(|(name, _)| name == "AppMsgId")
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    );
    eprintln!(
        "[wechat] 字段预览:\n{}",
        preview_fields(&fields, &["title0", "digest0", "content0"], 100)
    );

    let client = reqwest::Client::new();
    let response = match client
        .post(&url)
        .header("Cookie", &credentials.cookie)
        .header("User-Agent", USER_AGENT)
        .header("Referer", HOME_URL)
        .header("Origin", "https://mp.weixin.qq.com")
        .header("X-Requested-With", "XMLHttpRequest")
        .multipart(form)
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

    eprintln!("[wechat] 原始响应: {}", response_text);

    let parsed = parse_response(&response_text);

    if !parsed.success {
        let class = if status.as_u16() == 401 || status.as_u16() == 403 {
            FailureClass::AuthInvalid
        } else {
            FailureClass::PlatformError
        };
        let ret_label = match parsed.ret {
            Some(n) => n.to_string(),
            None => "无".to_string(),
        };
        return PublishResult::failure(
            class,
            format!(
                "微信公众号 API 错误 (ret: {}, HTTP {}): {}",
                ret_label, status, parsed.detail
            ),
            operation,
            false,
        );
    }

    let returned_id = match parsed.json.as_ref().and_then(extract_appmsg_id) {
        Some(id) => id,
        None if operation == PublishOperation::Update && !target_id.is_empty() => target_id.clone(),
        None => {
            return PublishResult::failure(
                FailureClass::PlatformError,
                format!(
                    "微信返回成功但未能提取 AppMsgId，原始响应: {}",
                    truncate::truncate_chars(&response_text, ERROR_SNIPPET_CHARS)
                ),
                operation,
                false,
            )
        }
    };

    let action = match operation {
        PublishOperation::Create => "草稿创建成功",
        PublishOperation::Update => "草稿更新成功",
    };
    let message = format!(
        "{}! AppMsgId: {}（请到公众号后台草稿箱确认）",
        action, returned_id
    );

    PublishResult::success(
        returned_id.clone(),
        build_edit_url(&returned_id, &credentials.token),
        message,
        operation,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> PublishRequest {
        PublishRequest {
            title: "测试标题".to_string(),
            content: "# 小标题\n\n正文第一行\n正文第二行".to_string(),
            ..Default::default()
        }
    }

    const TEST_TOKEN: &str = "1234567890";

    fn field_names(fields: &[(String, String)]) -> Vec<String> {
        fields.iter().map(|(name, _)| name.clone()).collect()
    }

    fn field_value<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn create_url_uses_sub_create() {
        let url = build_url("123456", PublishOperation::Create);
        assert!(url.contains("sub=create"), "{}", url);
        assert!(url.contains("t=ajax-response"));
        assert!(url.contains("type=77"));
        assert!(url.contains("token=123456"));
        assert!(url.contains("lang=zh_CN"));
    }

    #[test]
    fn update_url_uses_sub_update() {
        let url = build_url("123456", PublishOperation::Update);
        assert!(url.contains("sub=update"), "{}", url);
    }

    #[test]
    fn core_index_fields_are_present() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let names = field_names(&fields);
        for expected in [
            "title0",
            "content0",
            "digest0",
            "author0",
            "auto_gen_digest0",
            "sourceurl0",
        ] {
            assert!(names.contains(&expected.to_string()), "缺少 {}", expected);
        }
    }

    #[test]
    fn create_sends_empty_appmsgid() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "AppMsgId"), Some(""));
    }

    #[test]
    fn update_carries_appmsgid() {
        let fields = build_fields(&sample_request(), PublishOperation::Update, "100000038", TEST_TOKEN);
        assert_eq!(field_value(&fields, "AppMsgId"), Some("100000038"));
    }

    #[test]
    fn single_article_counter_is_one() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "count"), Some("1"));
    }

    #[test]
    fn articlenum_is_not_sent() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert!(
            !field_names(&fields).contains(&"articlenum".to_string()),
            "参考实现不含 articlenum"
        );
    }

    #[test]
    fn no_second_article_index_fields() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        for name in field_names(&fields) {
            assert!(
                !name.ends_with('1'),
                "不应出现 1 系列索引字段: {}",
                name
            );
        }
    }

    #[test]
    fn does_not_invent_unverified_multi_flags() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let names = field_names(&fields);
        assert!(!names.contains(&"is_multi".to_string()));
        assert!(!names.contains(&"multi_item".to_string()));
    }

    #[test]
    fn only_captured_static_fields_are_sent() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let names = field_names(&fields);
        let allowed_static: Vec<&str> = STATIC_FIELDS.iter().map(|(n, _)| *n).collect();
        for name in &names {
            let is_core = [
                "title0",
                "content0",
                "digest0",
                "author0",
                "auto_gen_digest0",
            ]
            .contains(&name.as_str());
            let is_dynamic = ["token", "AppMsgId", "random"].contains(&name.as_str());
            assert!(
                is_core || is_dynamic || allowed_static.contains(&name.as_str()),
                "出现了参考实现中不存在的字段: {}",
                name
            );
        }
    }

    #[test]
    fn token_is_sent_in_body() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "token"), Some(TEST_TOKEN));
    }

    #[test]
    fn capture_static_fields_are_present() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        for (name, expected) in [
            ("lang", "zh_CN"),
            ("f", "json"),
            ("ajax", "1"),
            ("operate_from", "Chrome"),
            ("isnew", "0"),
            ("show_cover_pic0", "0"),
            ("need_open_comment0", "1"),
            ("only_fans_can_comment0", "0"),
        ] {
            assert_eq!(field_value(&fields, name), Some(expected), "字段 {} 不符", name);
        }
    }

    #[test]
    fn fingerprint_is_not_sent() {
        // 参考实现从不发送 fingerprint；伪造的 32 位占位值会被判成参数错误
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert!(
            !field_names(&fields).contains(&"fingerprint".to_string()),
            "不应发送 fingerprint"
        );
    }

    #[test]
    fn random_field_is_a_decimal_fraction() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let random = field_value(&fields, "random").unwrap();
        assert!(random.starts_with("0."), "{}", random);
        assert!(random.chars().skip(2).all(|c| c.is_ascii_digit()), "{}", random);
    }

    #[test]
    fn sourceurl0_is_present_and_empty() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "sourceurl0"), Some(""));
    }

    #[test]
    fn auto_gen_digest_follows_digest_value() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "auto_gen_digest0"), Some("0"));

        let empty = PublishRequest {
            title: "t".to_string(),
            content: String::new(),
            ..Default::default()
        };
        let fields = build_fields(&empty, PublishOperation::Create, "", TEST_TOKEN);
        assert_eq!(field_value(&fields, "digest0"), Some(""));
        assert_eq!(field_value(&fields, "auto_gen_digest0"), Some("1"));
    }

    #[test]
    fn digest0_never_exceeds_wechat_limit() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "字".repeat(500),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let digest = field_value(&fields, "digest0").unwrap();
        assert!(digest.chars().count() <= DIGEST_LIMIT, "{}", digest.chars().count());
    }

    #[test]
    fn content0_drops_unsupported_tags() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "正文<script>alert(1)</script>中间<style>.a{}</style>结尾".to_string(),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(!content.contains("<script"), "{}", content);
        assert!(!content.contains("</script"), "{}", content);
        assert!(!content.contains("<style"), "{}", content);
        assert!(!content.contains("</style"), "{}", content);
        assert!(!content.contains("alert"), "{}", content);
        assert!(content.contains("正文"), "{}", content);
        assert!(content.contains("结尾"), "{}", content);
    }

    #[test]
    fn content0_escapes_ampersand_and_angle_brackets() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "a & b < c > d".to_string(),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(content.contains("&amp;"), "{}", content);
        assert!(content.contains("&lt;"), "{}", content);
        assert!(content.contains("&gt;"), "{}", content);
    }

    #[test]
    fn preview_log_exposes_first_chars_only() {
        let request = PublishRequest {
            title: "标题".repeat(80),
            content: "正文".repeat(80),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let preview = preview_fields(&fields, &["title0", "content0"], 100);
        assert!(preview.contains("title0="));
        assert!(preview.contains("content0="));
        // 前缀保留 100 个字符，后面最多再跟一段截断提示
        for line in preview.lines() {
            let value = line.splitn(2, '=').nth(1).unwrap_or("");
            assert!(
                value.chars().count() <= 100 + "...(已截断)".chars().count(),
                "{}",
                line
            );
        }
    }

    #[test]
    fn content0_ends_with_mp_style_type_tail() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(content.ends_with(CONTENT_TAIL), "{}", content);
        assert!(content.contains(r#"<mp-style-type data-value="3">"#), "{}", content);
    }

    #[test]
    fn content0_is_html() {
        let fields = build_fields(&sample_request(), PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(content.starts_with("<h1>"), "{}", content);
        assert!(!content.contains("![]("));
    }

    #[test]
    fn content0_replaces_image_syntax() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "前文\n![截图](https://example.com/a.png)\n后文".to_string(),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(content.contains("[图片]"), "{}", content);
        assert!(!content.contains("example.com"), "{}", content);
        assert!(!content.contains("!["), "{}", content);
    }

    #[test]
    fn content0_keeps_plain_links() {
        let request = PublishRequest {
            title: "t".to_string(),
            content: "参考 [文档](https://example.com/doc)".to_string(),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let content = field_value(&fields, "content0").unwrap();
        assert!(content.contains("文档"), "{}", content);
        assert!(!content.contains("<a"), "{}", content);
        assert!(!content.contains("href"), "{}", content);
        assert!(!content.contains("example.com"), "{}", content);
    }

    #[test]
    fn digest_strips_markdown_markup() {
        let digest = build_digest("# 标题\n\n**加粗** 与 `代码` 与 [链接](https://a.b)", DIGEST_LIMIT);
        assert!(!digest.contains('#'), "{}", digest);
        assert!(!digest.contains('*'), "{}", digest);
        assert!(!digest.contains('`'), "{}", digest);
        assert!(!digest.contains("https://"), "{}", digest);
        assert!(digest.contains("加粗"), "{}", digest);
        assert!(digest.contains("链接"), "{}", digest);
    }

    #[test]
    fn digest_drops_images_entirely() {
        let digest = build_digest("![图](https://a.b/c.png) 正文", DIGEST_LIMIT);
        assert_eq!(digest, "正文");
    }

    #[test]
    fn digest_truncates_to_limit() {
        let long = "字".repeat(200);
        let digest = build_digest(&long, DIGEST_LIMIT);
        assert_eq!(digest.chars().count(), DIGEST_LIMIT);
        assert!(digest.ends_with('…'));
    }

    #[test]
    fn digest_keeps_short_text_intact() {
        assert_eq!(build_digest("短摘要", DIGEST_LIMIT), "短摘要");
    }

    #[test]
    fn parses_success_response() {
        let parsed = parse_response(r#"{"ret":0,"errmsg":"ok","AppMsgId":100000038}"#);
        assert!(parsed.success);
        assert_eq!(parsed.ret, Some(0));
    }

    #[test]
    fn parses_business_failure() {
        let parsed = parse_response(r#"{"ret":200003,"errmsg":"invalid token"}"#);
        assert!(!parsed.success);
        assert_eq!(parsed.ret, Some(200003));
        assert!(parsed.detail.contains("invalid token"));
    }

    #[test]
    fn parses_failure_from_base_resp() {
        let parsed = parse_response(r#"{"base_resp":{"ret":-1,"errmsg":"system error"}}"#);
        assert!(!parsed.success);
        assert_eq!(parsed.ret, Some(-1));
        assert!(parsed.detail.contains("system error"));
    }

    #[test]
    fn non_json_body_is_failure() {
        let parsed = parse_response("<html>gateway error</html>");
        assert!(!parsed.success);
        assert_eq!(parsed.ret, None);
    }

    #[test]
    fn missing_ret_is_failure_not_success() {
        let parsed = parse_response(r#"{"errmsg":"ok"}"#);
        assert!(!parsed.success);
        assert_eq!(parsed.ret, None);
    }

    #[test]
    fn extracts_appmsg_id_from_various_shapes() {
        let direct = serde_json::json!({"AppMsgId": 100000038});
        assert_eq!(extract_appmsg_id(&direct), Some("100000038".to_string()));

        let string_form = serde_json::json!({"appMsgId": "100000038"});
        assert_eq!(
            extract_appmsg_id(&string_form),
            Some("100000038".to_string())
        );

        let nested = serde_json::json!({"data": {"appmsgid": 100000038}});
        assert_eq!(extract_appmsg_id(&nested), Some("100000038".to_string()));
    }

    #[test]
    fn placeholder_ids_are_filtered() {
        for value in [
            serde_json::json!({"AppMsgId": 0}),
            serde_json::json!({"AppMsgId": ""}),
            serde_json::json!({"AppMsgId": "未知"}),
        ] {
            assert_eq!(extract_appmsg_id(&value), None);
        }
    }

    #[test]
    fn publish_status_is_rejected() {
        let mut request = sample_request();
        request.publish_status = true;
        let err = check_operation(&request, PublishOperation::Create).unwrap_err();
        assert_eq!(err.0, FailureClass::NotImplemented);
    }

    #[test]
    fn existing_published_is_rejected() {
        let mut request = sample_request();
        request.existing_published = true;
        let err = check_operation(&request, PublishOperation::Update).unwrap_err();
        assert_eq!(err.0, FailureClass::NotImplemented);
    }

    #[test]
    fn draft_create_is_allowed() {
        assert!(check_operation(&sample_request(), PublishOperation::Create).is_ok());
    }

    #[test]
    fn field_log_does_not_leak_content() {
        let request = PublishRequest {
            title: "机密标题".to_string(),
            content: "机密正文内容".to_string(),
            ..Default::default()
        };
        let fields = build_fields(&request, PublishOperation::Create, "", TEST_TOKEN);
        let described = describe_fields(&fields);
        assert!(!described.contains("机密"));
        assert!(described.contains("title0(len="));
    }

    #[test]
    fn edit_url_points_to_wechat_editor() {
        let url = build_edit_url("100000038", "123456");
        assert!(url.contains("appmsgid=100000038"));
        assert!(url.contains("mp.weixin.qq.com"));
    }

    #[test]
    fn edit_url_falls_back_to_home_without_id() {
        assert_eq!(build_edit_url("", "123456"), HOME_URL);
    }
}