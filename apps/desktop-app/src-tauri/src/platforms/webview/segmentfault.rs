//! 思否通道适配器（纯 fetch 建草稿 + 发布）。
//!
//! 思否的核心写接口 `/gateway/draft`（建草稿）和 `/gateway/article`（发布）
//! **不需要 Gateway keyv+ivd 签名**——只有 `/gateway/tags` 这类辅助接口才需要。
//! 这两个接口只要求：
//! 1. `token` header = sessionToken（从 `/write` 页面 HTML 里正则提取）
//! 2. Cookie 在浏览器上下文里（WebView2 自动带上）
//!
//! 因此整个流程非常简洁：
//! 1. open_channel_window 到 `https://segmentfault.com/write`（写作页）
//! 2. 等页面加载完，注入脚本：
//!    a. 从当前页面 HTML 正则提取 sessionToken（`serverData.Token` 或
//!       `window.g_initialProps.global.sessionInfo.key`）
//!    b. `POST /gateway/draft` 建草稿（带 title + markdown text + tags）
//!    c. 如果勾选发布：`POST /gateway/article` 发布
//!    d. 通过 `document.title` 回传结果
//! 3. Rust 侧轮询 `window.title()` 拿到结果，`window.close()` 收尾。
//!
//! 全程可见窗口短暂弹出（写作页），自动完成后关闭。

use std::time::Duration;

use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, open_channel_window};
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "segmentfault";

pub const WRITE_URL: &str = "https://segmentfault.com/write";
pub const LOGIN_URL: &str = "https://segmentfault.com/";

pub const STATUS_PREFIX: &str = "OPENPUB: ";

const SCRIPT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 300;
const WINDOW_WAIT_MS: u64 = 3_000;

const LOGIN_CHECK_TIMEOUT_MS: u64 = 300_000;
const LOGIN_CHECK_INTERVAL_MS: u64 = 500;

pub fn is_login_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("/signin") || url.contains("/login")
}

pub fn is_logged_in_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    !url.contains("/signin") && !url.contains("/login")
        && (url.contains("/user/draft") || url.contains("/user/settings") || url.contains("/write"))
}

async fn ensure_login_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let label = channel_window_label(PLATFORM);

    if let Some(existing) = app.get_webview_window(&label) {
        let url = existing.url().map(|u| u.to_string()).unwrap_or_default();
        if is_logged_in_url(&url) {
            return Ok(existing);
        }
        let _ = existing.close();
    }

    let window = open_channel_window(app, PLATFORM, WRITE_URL)?;

    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    while Instant::now() < deadline {
        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if is_logged_in_url(&url) {
            sleep(Duration::from_millis(WINDOW_WAIT_MS)).await;
            return Ok(window);
        }
        if is_login_url(&url) {
            sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
            continue;
        }
        if !url.is_empty() && !url.contains("segmentfault") {
            sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
            continue;
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    Err("等待思否登录超时".to_string())
}

fn parse_status(title: &str) -> (String, String) {
    if let Some(rest) = title.strip_prefix(STATUS_PREFIX) {
        if let Some(colon_idx) = rest.find(':') {
            let kind = rest[..colon_idx].trim().to_string();
            let payload = rest[colon_idx + 1..].trim().to_string();
            return (kind, payload);
        }
    }
    ("UNKNOWN".to_string(), title.to_string())
}

fn build_fetch_script(params_json: &str) -> String {
    let prefix = STATUS_PREFIX;
    format!(
        r#"(function() {{
  var PREFIX = "{prefix}";
  var P = {params_json};
  var F = "https://segmentfault.com";

  function sleep(ms) {{ return new Promise(function(r) {{ setTimeout(r, ms); }}); }}

  function extractToken(html) {{
    var m = html.match(/"Token"\s*:\s*"([^"]+)"/);
    if (m && m[1]) return m[1];
    try {{
      var idx = html.indexOf("window.g_initialProps");
      if (idx >= 0) {{
        var end = html.indexOf("</script>", idx);
        if (end > idx) {{
          var snippet = html.slice(idx + "window.g_initialProps".length, end).trim().replace(/;\s*$/, "");
          var obj = JSON.parse(snippet);
          var sess = obj && obj.global && obj.global.sessionInfo;
          if (sess && sess.key) return sess.key;
        }}
      }}
    }} catch(e) {{}}
    return null;
  }}

  async function run() {{
    try {{
      var html = document.documentElement.outerHTML;
      var token = extractToken(html);
      if (!token) {{
        var resp = await fetch(F + "/write", {{ credentials: "include" }});
        html = await resp.text();
        token = extractToken(html);
      }}
      if (!token) {{
        document.title = PREFIX + "FAIL:思否 sessionToken 未找到，请刷新写作页重试";
        return;
      }}

      var textBody = P.markdown || P.content || "";

      var draftResp = await fetch(F + "/gateway/draft", {{
        method: "POST",
        credentials: "include",
        headers: {{
          "Content-Type": "application/json",
          "accept": "*/*",
          "token": token
        }},
        body: JSON.stringify({{
          title: P.title,
          tags: [],
          text: textBody,
          object_id: "",
          type: "article"
        }})
      }});
      var draftText = await draftResp.text();
      var draftId = null;
      try {{
        var draftJson = JSON.parse(draftText);
        if (Array.isArray(draftJson)) {{
          if (draftJson[0] === 0 && draftJson[1] && draftJson[1].id) {{
            draftId = String(draftJson[1].id);
          }}
        }} else if (draftJson && draftJson.id) {{
          draftId = String(draftJson.id);
        }}
      }} catch(e) {{}}

      if (!draftResp.ok || !draftId) {{
        document.title = PREFIX + "FAIL:建草稿失败 HTTP " + draftResp.status + " " + draftText.slice(0, 200);
        return;
      }}

      var articleId = null;
      if (P.publish && !P.existing_published) {{
        var blogId = P.blog_id || "0";
        var pubResp = await fetch(F + "/gateway/article", {{
          method: "POST",
          credentials: "include",
          headers: {{
            "Content-Type": "application/json",
            "accept": "application/json",
            "token": token
          }},
          body: JSON.stringify({{
            tags: [],
            title: P.title,
            text: textBody,
            draft_id: draftId,
            blog_id: blogId,
            type: 1,
            url: "",
            cover: "",
            license: null,
            log: ""
          }})
        }});
        var pubText = await pubResp.text();
        try {{
          var pubJson = JSON.parse(pubText);
          if (pubJson && pubJson.data && pubJson.data.id) {{
            articleId = String(pubJson.data.id);
          }} else if (pubJson && pubJson.id) {{
            articleId = String(pubJson.id);
          }}
        }} catch(e) {{}}

        if (!pubResp.ok) {{
          document.title = PREFIX + "FAIL:发布失败 HTTP " + pubResp.status + " " + pubText.slice(0, 200);
          return;
        }}
      }}

      document.title = PREFIX + "DONE:" + JSON.stringify({{
        draftId: draftId,
        articleId: articleId
      }});
    }} catch(err) {{
      document.title = PREFIX + "FAIL:" + String((err && err.message) || err);
    }}
  }}
  run();
}})();"#,
        prefix = prefix,
        params_json = params_json,
    )
}

async fn run_fetch_phase(
    window: &tauri::WebviewWindow,
    params_json: &str,
) -> Result<(Option<String>, Option<String>), String> {
    let script = build_fetch_script(params_json);
    if let Err(e) = window.eval(&script) {
        return Err(format!("脚本注入失败：{}", e));
    }

    let mut last_title = String::new();
    let deadline = Instant::now() + Duration::from_millis(SCRIPT_TIMEOUT_MS);

    while Instant::now() < deadline {
        let title = window.title().unwrap_or_default();
        if let Some(status) = title.strip_prefix(STATUS_PREFIX) {
            if status != last_title.as_str() {
                let (kind, payload) = parse_status(status);
                match kind.as_str() {
                    "DONE" => {
                        let parsed: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| "结果 JSON 解析失败".to_string())?;
                        let draft_id = parsed.get("draftId").and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let article_id = parsed.get("articleId").and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        return Ok((draft_id, article_id));
                    }
                    "FAIL" => return Err(payload.to_string()),
                    _ => return Err(format!("未知状态：{}", kind)),
                }
            }
        }
        last_title = title;
        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    Err("思否脚本执行超时".to_string())
}

pub async fn run(
    app: &AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let start = Instant::now();

    if request.title.trim().is_empty() {
        return delay_return(start, PublishResult::failure(
            FailureClass::Unknown, "标题不能为空".to_string(), operation, false,
        ));
    }

    let existing_id = request.existing_id.clone();
    let existing_published = request.existing_published;

    let window = match ensure_login_window(app).await {
        Ok(w) => w,
        Err(e) => {
            return delay_return(start, PublishResult::failure(
                FailureClass::AuthInvalid,
                format!("思否登录失败：{}", e),
                operation, false,
            ));
        }
    };

    let title = request.title.trim().to_string();
    let content = request.content.clone();

    let params = serde_json::json!({
        "title": title,
        "content": content,
        "publish": request.publish_status,
        "existing_published": existing_published,
        "existing_id": existing_id.as_deref().unwrap_or("")
    });

    let fetch_result = run_fetch_phase(&window, &params.to_string()).await;
    let (draft_id, article_id) = match fetch_result {
        Ok(v) => v,
        Err(e) => {
            let _ = window.close();
            return delay_return(start, PublishResult::failure(
                FailureClass::PlatformError,
                format!("思否发布失败：{}", e),
                operation, false,
            ));
        }
    };

    let _ = window.close();

    let article_id_final = article_id.clone()
        .or_else(|| draft_id.clone())
        .unwrap_or_default();

    let url = if let Some(ref aid) = article_id {
        format!("https://segmentfault.com/a/{}", aid)
    } else if let Some(ref did) = draft_id {
        format!("https://segmentfault.com/write?draftId={}", did)
    } else {
        String::new()
    };

    let msg = if let Some(ref aid) = article_id {
        format!("思否已发布（https://segmentfault.com/a/{}）", aid)
    } else if let Some(ref did) = draft_id {
        format!("思否草稿已保存（https://segmentfault.com/write?draftId={}）", did)
    } else {
        "思否操作完成但未获取到 ID".to_string()
    };

    delay_return(start, PublishResult::success(
        article_id_final,
        url,
        msg,
        operation,
        article_id.is_some(),
    ))
}

fn delay_return(start: Instant, result: PublishResult) -> PublishResult {
    let min_latency = Duration::from_millis(600);
    let elapsed = start.elapsed();
    if elapsed < min_latency {
        std::thread::sleep(min_latency - elapsed);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_detection() {
        assert!(is_login_url("https://segmentfault.com/signin"));
        assert!(is_login_url("https://segmentfault.com/login"));
        assert!(!is_login_url("https://segmentfault.com/write"));
        assert!(!is_login_url("https://segmentfault.com/"));
    }

    #[test]
    fn logged_in_url_detection() {
        assert!(is_logged_in_url("https://segmentfault.com/user/draft"));
        assert!(is_logged_in_url("https://segmentfault.com/user/settings"));
        assert!(is_logged_in_url("https://segmentfault.com/write"));
        assert!(!is_logged_in_url("https://segmentfault.com/signin"));
        assert!(!is_logged_in_url("https://segmentfault.com/"));
    }

    #[test]
    fn status_prefix_parsing() {
        let (kind, payload) = parse_status("OPENPUB: DONE:{\"draftId\":\"123\"}");
        assert_eq!(kind, "DONE");
        assert_eq!(payload, "{\"draftId\":\"123\"}");

        let (k2, p2) = parse_status("OPENPUB: FAIL:token not found");
        assert_eq!(k2, "FAIL");
        assert_eq!(p2, "token not found");

        let (k3, p3) = parse_status("just a title");
        assert_eq!(k3, "UNKNOWN");
        assert_eq!(p3, "just a title");
    }

    #[test]
    fn fetch_script_contains_key_endpoints() {
        let params = serde_json::json!({
            "title": "test",
            "content": "# hello",
            "publish": false,
            "existing_published": false,
            "existing_id": ""
        });
        let script = build_fetch_script(&params.to_string());
        assert!(script.contains("/gateway/draft"));
        assert!(script.contains("\"token\""));
        assert!(script.contains("OPENPUB:"));
    }

    #[test]
    fn fetch_script_contains_publish_endpoint_when_requested() {
        let params = serde_json::json!({
            "title": "test",
            "markdown": "# hello",
            "publish": true,
            "existing_published": false
        });
        let script = build_fetch_script(&params.to_string());
        assert!(script.contains("/gateway/article"));
    }
}