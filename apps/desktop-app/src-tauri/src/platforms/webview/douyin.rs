//! 抖音通道适配器（页面上下文内 fetch 保存草稿）。
//!
//! 抖音创作者中心的长文草稿接口 `/web/api/media/aweme/draft?aid=1128`
//! **不做运行时签名**，但它把「登录态 + 站点特征」一起校验：Rust 侧 `reqwest`
//! 直发即使带上完整 Cookie 也会被当成未登录 / 异常请求。原始插件
//! `docs/index.mjs` 的 `savePreparedDraft` 是通过 `executeInDouyinTab` 在
//! **创作者中心长文编辑页自己的上下文**里发 `fetch`，浏览器自动带上的
//! `Origin` / `Referer` / 页面侧指纹与请求天然一致，因此不会被判伪造。
//!
//! 这里照搬同一模型（与 `toutiao.rs` 完全同构）：
//! 1. `open_channel_window` 到
//!    `https://creator.douyin.com/creator-micro/content/post/article`
//! 2. 等页面落到 `creator-micro` 站内路由（未登录会回到 `creator.douyin.com` 首页
//!    的扫码登录页），注入脚本：
//!    a. 用原始插件的 `buildDraftPayload` 字段集 `POST` 草稿接口
//!    b. 勾选发布也**只存草稿**——抖音的 `create_v2` 发布接口要求 `media_type`、
//!       `图文/长文` 分支参数与封面素材，缺一即报错；草稿保存已在后台可见，
//!       发布留给用户在创作者中心点一次，比在这里猜参数更可靠
//!    c. 结果同时写 `document.title` 与 URL hash
//! 3. Rust 侧轮询两个通道取回结果，`window.close()` 收尾。
//!
//! 草稿 payload 在 Rust 侧拼装（字段名与原始插件逐字一致），只有 `creation_id`
//! 与 `init_timestamp` 由调用方决定：
//! - 新建：随机 8 位 + 毫秒时间戳（与插件 `createCreationId` 同算法）
//! - 更新：复用同步表里的 `creation_id`，让草稿在抖音后台被覆盖而不是复制一份

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, open_channel_window, open_channel_window_with_visibility};
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "douyin";

/// 抖音创作者中心长文编辑页。脚本必须在它自己的页面上下文里执行。
pub const EDITOR_URL: &str = "https://creator.douyin.com/creator-micro/content/post/article";
/// 草稿保存接口，与原始插件 `savePreparedDraft` 完全一致（`aid=1128` 是长文渠道）。
const DRAFT_URL: &str = "https://creator.douyin.com/web/api/media/aweme/draft?aid=1128";

pub const STATUS_PREFIX: &str = "OPENPUB: ";

const SCRIPT_TIMEOUT_MS: u64 = 30_000;
const POLL_INTERVAL_MS: u64 = 250;
const REINJECT_AFTER_MS: u64 = 3_000;
const MAX_INJECT_ATTEMPTS: u32 = 4;

/// 结果 hash 的键名。脚本侧写的是 `#openpub_res=<encodeURIComponent(state)>`。
const RESULT_HASH_KEY: &str = "openpub_res=";

/// 每次新的保存尝试注入前先清掉上一轮的会话标记。
///
/// 脚本用 `sessionStorage` 记住「已经发起过」和「上一轮终态」，防止重注入时
/// 二次建草稿；但若不清，新的保存尝试会把上一轮的 `FAIL` 直接回放成这一轮的结果。
const CLEAR_SESSION_SCRIPT: &str = "try { sessionStorage.removeItem('__openpub_dy_state__'); sessionStorage.removeItem('__openpub_dy_started__'); } catch (e) {}";

const LOGIN_CHECK_TIMEOUT_MS: u64 = 300_000;
const LOGIN_CHECK_INTERVAL_MS: u64 = 500;
const WINDOW_WAIT_MS: u64 = 3_000;

/// 隐藏窗口静默探测登录态的时长，与头条通道保持一致。
const SILENT_PROBE_MS: u64 = 8_000;

/// 判定「这个 URL 处于登录 / 风控流程中，绝不能当成已登录」。
///
/// 抖音的登录域是 `sso.douyin.com` / `passport.douyin.com`，未登录访问
/// `creator.douyin.com/creator-micro/*` 会被顶回 `creator.douyin.com` 首页的
/// 扫码登录页（该页 URL 里没有 `creator-micro`）。三类都要排除。
pub fn is_login_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("sso.")
        || url.contains("passport")
        || url.contains("/login")
        || url.contains("/verify")
}

/// 已登录判据：落在 `creator.douyin.com` 的 `creator-micro` 站内路由上。
///
/// **不能**用「URL 含 `creator.douyin.com`」就判已登录——未登录时的扫码登录页
/// 同样挂在这个域名下，那样会在窗口刚打开时就把用户标成已登录并关窗。
pub fn is_logged_in_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("creator.douyin.com") && url.contains("/creator-micro/") && !is_login_url(&url)
}

/// 严格判定「当前窗口确实停在抖音长文编辑页」。
///
/// 只有这个页面上注入的 `fetch` 才是同源请求；停在扫码登录页时请求目标是跨域，
/// 必报 CORS，表现成「保存失败」但根因是没登录，因此单独给一个谓词。
pub fn is_editor_page(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("creator.douyin.com")
        && !is_login_url(&url)
        && url.contains("/creator-micro/content/post/article")
}

/// 与原始插件 `createCreationId` 同算法：8 位随机小写字母 / 数字 + 毫秒时间戳。
///
/// 项目没有随机数依赖，用一个时间种子的 xorshift 足够——这个值只需在同一毫秒
/// 内不重复，服务端并不校验它的随机性。
fn create_creation_id() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut seed = if nanos == 0 { 0x9E37_79B9_7F4A_7C15 } else { nanos };
    let mut suffix = String::with_capacity(8);
    for _ in 0..8 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        suffix.push(CHARSET[(seed % CHARSET.len() as u64) as usize] as char);
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{}{}", suffix, millis)
}

fn init_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn edit_url(creation_id: &str, timestamp: u64) -> String {
    format!(
        "{}?enter_from=draft&creation_id={}&init_timestamp={}",
        EDITOR_URL, creation_id, timestamp
    )
}

async fn ensure_login_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let label = channel_window_label(PLATFORM);

    // 已有窗口一律复用，绝不关窗重开：抖音的登录流程会跳到 `sso.douyin.com`
    // 的扫码页，关掉就等于把用户刚完成的授权结果一起丢掉。
    let window = match app.get_webview_window(&label) {
        Some(existing) => {
            let url = existing.url().map(|u| u.to_string()).unwrap_or_default();
            if !is_logged_in_url(&url) {
                let _ = existing.unminimize();
                let _ = existing.show();
                let _ = existing.set_focus();
            }
            existing
        }
        None => {
            // 静默探测：先隐藏建窗。Cookie 仍在 WebView2 profile 里时，编辑页会在
            // 几秒内直接落地，用户全程看不到窗口闪一下；落不了地才显示出来让用户扫码。
            let window = open_channel_window_with_visibility(app, PLATFORM, EDITOR_URL, false)?;
            let probe_deadline = Instant::now() + Duration::from_millis(SILENT_PROBE_MS);
            loop {
                if app.get_webview_window(&label).is_none() {
                    return Err("抖音登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
                }
                let url = window.url().map(|u| u.to_string()).unwrap_or_default();
                if is_logged_in_url(&url) {
                    break;
                }
                if Instant::now() >= probe_deadline {
                    eprintln!(
                        "[douyin] 静默探测未确认登录，显示窗口等待用户处理，当前页面: {}",
                        url
                    );
                    let _ = window.show();
                    let _ = window.set_focus();
                    break;
                }
                sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
            }
            window
        }
    };

    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    let mut last_url = String::new();
    while Instant::now() < deadline {
        if app.get_webview_window(&label).is_none() {
            return Err("抖音登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
        }

        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url != last_url {
            eprintln!("[douyin] 登录窗口页面变化: {}", url);
            last_url = url.clone();
        }
        if is_logged_in_url(&url) {
            ensure_editor_page(&window).await?;
            return Ok(window);
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    Err(format!(
        "等待抖音登录超时，请在通道窗口里完成登录后重试（当前页面: {}）",
        window.url().map(|u| u.to_string()).unwrap_or_default()
    ))
}

/// 确保窗口停在抖音长文编辑页，否则导航过去并等待落地。
async fn ensure_editor_page(window: &tauri::WebviewWindow) -> Result<(), String> {
    let current = window.url().map(|u| u.to_string()).unwrap_or_default();
    if is_editor_page(&current) {
        sleep(Duration::from_millis(WINDOW_WAIT_MS)).await;
        return Ok(());
    }

    let script = super::navigate_script(EDITOR_URL)?;
    let _ = window.eval(&script);

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if is_editor_page(&url) {
            sleep(Duration::from_millis(WINDOW_WAIT_MS)).await;
            return Ok(());
        }
        if is_login_url(&url) {
            return Err(format!(
                "抖音要求先完成登录（当前页面: {}），请在通道窗口里处理后重试",
                url
            ));
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    Err(format!(
        "打开抖音长文编辑页超时，当前页面: {}",
        window.url().map(|u| u.to_string()).unwrap_or_default()
    ))
}

/// 前端「打开平台页面」入口：拉起通道窗口到长文编辑页。
pub async fn open_login_channel(app: &AppHandle) -> Result<(), String> {
    let label = channel_window_label(PLATFORM);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
    } else {
        open_channel_window(app, PLATFORM, EDITOR_URL)?;
    }

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        watch_login(&app_handle).await;
    });
    Ok(())
}

async fn watch_login(app: &AppHandle) {
    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    let mut last_url = String::new();

    loop {
        if Instant::now() >= deadline {
            // 与思否同理：抖音登录会跳 `sso.douyin.com` 扫码页，超时关窗会把
            // 用户刚完成的扫码结果一起丢掉，只回传失败释放前端锁即可。
            crate::login::emit_failed(
                app,
                PLATFORM,
                "等待抖音登录超时，窗口已保留；完成登录后请重新同步",
            );
            return;
        }

        let Some(window) = app.get_webview_window(&label) else {
            crate::login::emit_failed(app, PLATFORM, "登录窗口已关闭，未完成登录");
            return;
        };

        if let Ok(url) = window.url() {
            let url = url.to_string();
            if url != last_url {
                eprintln!("[douyin] 登录页变化: {}", url);
                last_url = url.clone();
            }
            if is_logged_in_url(&url) {
                crate::login::emit_success(app, PLATFORM);
                // `watch_login` 只服务于前端「打开平台页面」按钮（一键同步走
                // `run`，不经过这里），登录成功即关窗，否则窗口会一直悬在
                // 创作者中心首页，用户会以为程序卡住了。
                let _ = window.close();
                return;
            }
        }

        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }
}

fn build_draft_script(params_json: &str) -> String {
    let template = r##"(function () {
  var PREFIX = "__OPENPUB_PREFIX__";
  var P = __OPENPUB_PARAMS__;
  var STATE_KEY = "__openpub_dy_state__";
  var STARTED_KEY = "__openpub_dy_started__";
  var HASH_KEY = "__OPENPUB_HASH_KEY__";

  function isTerminal(text) {
    return text.indexOf("DONE:") === 0 || text.indexOf("FAIL:") === 0 || text.indexOf("AUTH:") === 0;
  }

  function writeHash(text) {
    try {
      var target = "#" + HASH_KEY + encodeURIComponent(text);
      if (location.hash !== target) { location.hash = target; }
    } catch (e) {}
  }

  function publish(text) {
    try { document.title = PREFIX + text; } catch (e) {}
    if (isTerminal(text)) { writeHash(text); }
  }

  function setStatus(text) {
    try { sessionStorage.setItem(STATE_KEY, text); } catch (e) {}
    publish(text);
  }

  // 页面若周期性重置标题或清掉 hash，靠这个心跳把终态重新写回去。
  setInterval(function () {
    var text = null;
    try { text = sessionStorage.getItem(STATE_KEY); } catch (e) {}
    if (text && isTerminal(text)) { publish(text); }
  }, 500);

  async function run() {
    var previous = null;
    try { previous = sessionStorage.getItem(STATE_KEY); } catch (e) {}
    if (previous) {
      publish(previous);
      if (isTerminal(previous)) { return; }
    }

    var started = false;
    try { started = sessionStorage.getItem(STARTED_KEY) === "1"; } catch (e) {}
    if (started) {
      // 已经发起过但还没拿到结果：只回放进度，绝不重跑，否则会重复建草稿。
      publish("RUNNING");
      return;
    }
    try { sessionStorage.setItem(STARTED_KEY, "1"); } catch (e) {}

    try {
      setStatus("RUNNING");

      var payload = {
        item: {
          common: {
            draft: {
              title: P.title,
              description: "",
              long_article: P.markdown,
              image_info: [],
              head_poster: "",
              poster: "",
              text_extra: "[]",
              visibility_type: 0,
              timing: 0,
              creation_id: P.creationId,
              init_timestamp: P.initTimestamp,
              req_type: 0
            }
          },
          cover: {}
        }
      };

      var response = await fetch(P.draftUrl, {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload)
      });

      // 抖音风控不走 HTTP 状态码，而是回这两个响应头让前端自行跳验证页。
      var decision = response.headers.get("x-tt-verify-passport-decision");
      var gateway = response.headers.get("bd_passport_security_gateway");
      if (decision || gateway === "1") {
        setStatus("AUTH:抖音要求账号安全验证（" + (decision || "security_gateway") + "），请在该窗口完成验证后重试");
        return;
      }

      var text = await response.text();
      if (!response.ok) {
        setStatus("FAIL:保存草稿失败 HTTP " + response.status + " " + text.slice(0, 200));
        return;
      }

      var json = null;
      try { json = JSON.parse(text); } catch (e) {}
      if (!json) {
        setStatus("FAIL:保存草稿响应不是 JSON: " + text.slice(0, 200));
        return;
      }
      if (json.status_code !== 0) {
        setStatus("FAIL:" + (json.status_msg || "保存抖音草稿失败") + " (status_code=" + json.status_code + ") " + text.slice(0, 200));
        return;
      }

      setStatus("DONE:" + JSON.stringify({ creationId: P.creationId, editUrl: P.editUrl }));
    } catch (err) {
      var msg = String((err && err.message) || err);
      // 浏览器对跨域拦截 / 风控拒绝只给一句笼统的 "Failed to fetch"，
      // 这里补上当前页面，否则日志里看不出到底是在哪个页面上发的请求。
      if (msg === "Failed to fetch" || msg === "Load failed") {
        msg = "页面内请求被拦截（跨域或风控），当前页面 " + location.href;
      }
      setStatus("FAIL:" + msg);
    }
  }

  run();
})();"##;

    template
        .replace("__OPENPUB_PREFIX__", STATUS_PREFIX)
        .replace("__OPENPUB_HASH_KEY__", RESULT_HASH_KEY)
        .replace("__OPENPUB_PARAMS__", params_json)
}

/// 终态结果。`Auth` 单独一类：它表示「窗口还在登录 / 验证页」，调用方要把它
/// 映射成 `AuthInvalid`（并保留窗口让用户就地处理），而不是笼统的平台错误。
enum DouyinTerminal {
    Done { creation_id: String, edit_url: String },
    Auth(String),
    Fail(String),
}

fn split_status(body: &str) -> (String, String) {
    if let Some(colon_idx) = body.find(':') {
        let kind = body[..colon_idx].trim().to_string();
        let payload = body[colon_idx + 1..].trim().to_string();
        return (kind, payload);
    }
    // 无冒号是进度心跳（`OPENPUB: RUNNING`）。保留原样作为状态名，
    // 由调用方判定是否终态，而不是在这里统一抹成 `UNKNOWN`。
    (body.trim().to_string(), String::new())
}

fn terminal_result(body: &str) -> Option<DouyinTerminal> {
    let (kind, payload) = split_status(body);
    match kind.as_str() {
        "DONE" => {
            let parsed: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(value) => value,
                Err(_) => return Some(DouyinTerminal::Fail(format!(
                    "结果 JSON 解析失败，原始内容: {}",
                    payload
                ))),
            };
            let creation_id = parsed
                .get("creationId")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            if creation_id.is_empty() {
                return Some(DouyinTerminal::Fail(format!(
                    "结果缺少草稿 ID，原始内容: {}",
                    payload
                )));
            }
            let edit_url = parsed
                .get("editUrl")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            Some(DouyinTerminal::Done { creation_id, edit_url })
        }
        "AUTH" => Some(DouyinTerminal::Auth(payload)),
        "FAIL" => Some(DouyinTerminal::Fail(payload)),
        _ => None,
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 解 `encodeURIComponent` 产生的百分号编码（与 `toutiao.rs` 同一实现）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 从窗口 URL 的 hash 取出脚本写的结果体（形如 `DONE:{...}` / `AUTH:...`）。
fn status_from_url(url: &str) -> Option<String> {
    let index = url.find(RESULT_HASH_KEY)?;
    let rest = &url[index + RESULT_HASH_KEY.len()..];
    let raw = rest.split('&').next().unwrap_or(rest);
    if raw.is_empty() {
        return None;
    }
    Some(percent_decode(raw))
}

/// 同时读 URL hash 与窗口标题两个通道。
///
/// 返回 `(是否看到脚本写的状态, 终态)`。两个都要读：hash 抗标题抢占，
/// 标题抗页面清空 hash，任一命中即可。
fn read_status_channels(
    window: &tauri::WebviewWindow,
) -> (bool, Option<DouyinTerminal>) {
    let mut saw = false;
    let mut terminal = None;

    let url = window.url().map(|url| url.to_string()).unwrap_or_default();
    if let Some(body) = status_from_url(&url) {
        saw = true;
        terminal = terminal_result(&body);
    }

    let title = window.title().unwrap_or_default();
    if let Some(rest) = title.strip_prefix(STATUS_PREFIX) {
        saw = true;
        if terminal.is_none() {
            terminal = terminal_result(rest);
        }
    }

    (saw, terminal)
}

/// 通道层错误：`Auth` 与其它失败分开，便于 `run` 映射成 `AuthInvalid`。
enum ChannelError {
    Auth(String),
    Fail(String),
}

async fn run_fetch_phase(
    window: &tauri::WebviewWindow,
    script: &str,
) -> Result<(String, String), ChannelError> {
    let _ = window.eval(CLEAR_SESSION_SCRIPT);

    if let Err(error) = window.eval(script) {
        return Err(ChannelError::Fail(format!("脚本注入失败：{}", error)));
    }

    let mut attempts: u32 = 1;
    let mut last_status_at = Instant::now();
    let mut last_title = String::new();
    let deadline = Instant::now() + Duration::from_millis(SCRIPT_TIMEOUT_MS);

    while Instant::now() < deadline {
        let (saw_status, terminal) = read_status_channels(window);
        match terminal {
            Some(DouyinTerminal::Done { creation_id, edit_url }) => {
                return Ok((creation_id, edit_url));
            }
            Some(DouyinTerminal::Auth(message)) => return Err(ChannelError::Auth(message)),
            Some(DouyinTerminal::Fail(message)) => return Err(ChannelError::Fail(message)),
            None => {}
        }
        if saw_status {
            // 还能看到脚本写的状态，说明脚本活着，不需要重注入。
            last_status_at = Instant::now();
        }

        let title = window.title().unwrap_or_default();
        if title != last_title {
            eprintln!("[douyin] 窗口标题变化: {}", title);
            last_title = title;
        }

        // 抖音编辑页是 SPA，加载中途的跳转会清掉已注入的脚本。若一段时间看不到
        // 状态前缀，说明脚本已丢失，重新注入一次；同一页面内重复注入会被
        // `sessionStorage` 的幂等标记挡掉，不会双发请求。
        if attempts < MAX_INJECT_ATTEMPTS
            && last_status_at.elapsed() >= Duration::from_millis(REINJECT_AFTER_MS)
        {
            let url = window.url().map(|u| u.to_string()).unwrap_or_default();
            eprintln!("[douyin] 第 {} 次重注入脚本，当前页面: {}", attempts + 1, url);
            let _ = window.eval(script);
            attempts += 1;
            last_status_at = Instant::now();
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }

    // 退出循环后再读一次：终态可能刚好落在最后一次轮询之后。
    match read_status_channels(window).1 {
        Some(DouyinTerminal::Done { creation_id, edit_url }) => return Ok((creation_id, edit_url)),
        Some(DouyinTerminal::Auth(message)) => return Err(ChannelError::Auth(message)),
        Some(DouyinTerminal::Fail(message)) => return Err(ChannelError::Fail(message)),
        None => {}
    }

    let current_title = window.title().unwrap_or_default();
    let current_url = window.url().map(|u| u.to_string()).unwrap_or_default();
    Err(ChannelError::Fail(format!(
        "等待抖音页面返回结果超时（{} 秒），当前窗口标题: {}，当前页面: {}",
        SCRIPT_TIMEOUT_MS / 1000,
        current_title,
        current_url
    )))
}

fn delay_return(start: Instant, result: PublishResult) -> PublishResult {
    let min_latency = Duration::from_millis(600);
    let elapsed = start.elapsed();
    if elapsed < min_latency {
        std::thread::sleep(min_latency - elapsed);
    }
    result
}

pub async fn run(
    app: &AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    let start = Instant::now();

    if request.title.trim().is_empty() {
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::Unknown,
                "标题不能为空".to_string(),
                operation,
                false,
            ),
        );
    }

    let timestamp = init_timestamp();
    // 新建用随机 ID；更新复用同步表里的 ID，让抖音后台覆盖同一篇草稿。
    let creation_id = match operation {
        PublishOperation::Create => create_creation_id(),
        PublishOperation::Update => match request
            .existing_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => id.to_string(),
            None => {
                return delay_return(
                    start,
                    PublishResult::failure(
                        FailureClass::NotConfigured,
                        "抖音更新草稿时缺少草稿 ID，请重新同步一次".to_string(),
                        operation,
                        false,
                    ),
                )
            }
        },
    };
    let target_edit_url = edit_url(&creation_id, timestamp);

    let window = match ensure_login_window(app).await {
        Ok(window) => window,
        Err(error) => {
            return delay_return(
                start,
                PublishResult::failure(
                    FailureClass::AuthInvalid,
                    format!("抖音登录失败：{}", error),
                    operation,
                    false,
                ),
            )
        }
    };

    let params = json!({
        "draftUrl": DRAFT_URL,
        "title": request.title.trim(),
        "markdown": request.content,
        "creationId": creation_id,
        "initTimestamp": timestamp,
        "editUrl": target_edit_url,
    });
    let script = build_draft_script(&params.to_string());

    // 注入前最后一道闸门：窗口仍可能被抖音风控异步顶到扫码 / 验证页，
    // 在那种页面上执行 `fetch` 是跨域请求，只会拿到 CORS 错误。
    let url_before_inject = window.url().map(|u| u.to_string()).unwrap_or_default();
    eprintln!("[douyin] 注入前页面: {}", url_before_inject);
    if !is_editor_page(&url_before_inject) {
        if is_login_url(&url_before_inject) {
            let _ = window.show();
            let _ = window.set_focus();
        } else {
            let _ = window.close();
        }
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::AuthInvalid,
                format!(
                    "抖音未停在长文编辑页，请在通道窗口里完成登录或账号验证后重试（当前页面: {}）",
                    url_before_inject
                ),
                operation,
                false,
            ),
        );
    }

    let fetched = run_fetch_phase(&window, &script).await;
    let (returned_id, returned_edit_url) = match fetched {
        Ok(value) => value,
        Err(ChannelError::Auth(message)) => {
            let url = window.url().map(|u| u.to_string()).unwrap_or_default();
            if is_login_url(&url) {
                let _ = window.show();
                let _ = window.set_focus();
            } else {
                let _ = window.close();
            }
            return delay_return(
                start,
                PublishResult::failure(
                    FailureClass::AuthInvalid,
                    format!("抖音要求账号安全验证：{}", message),
                    operation,
                    false,
                ),
            );
        }
        Err(ChannelError::Fail(error)) => {
            // 失败时若窗口已被顶到登录 / 验证页，保留窗口让用户就地处理，
            // 否则关掉——留着只会是一个没法用的页面。
            let url = window.url().map(|u| u.to_string()).unwrap_or_default();
            if is_login_url(&url) {
                let _ = window.show();
                let _ = window.set_focus();
            } else {
                let _ = window.close();
            }
            return delay_return(
                start,
                PublishResult::failure(
                    FailureClass::PlatformError,
                    format!("抖音保存失败：{}", error),
                    operation,
                    false,
                ),
            );
        }
    };

    let _ = window.close();

    // 更新时服务端若回了不同的 ID，说明它把这次更新当成新建了；静默采纳会
    // 污染同步表，必须报错让人到后台确认。
    if operation == PublishOperation::Update && returned_id != creation_id {
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::PlatformError,
                format!(
                    "草稿更新结果不一致：提交的 ID 为 {}，服务端返回 {}，请到抖音后台确认",
                    creation_id, returned_id
                ),
                operation,
                false,
            ),
        );
    }

    let final_edit_url = if returned_edit_url.is_empty() {
        target_edit_url
    } else {
        returned_edit_url
    };

    // 抖音这里只做草稿保存：发布接口 `create_v2` 还要 `media_type`、封面素材与
    // 长文分支参数，任何一项对不上都会报参数错误。勾了「发布」也如实告诉用户
    // 「草稿已保存」，`is_published` 保持 false，不谎报已发布。
    let message = if request.publish_status {
        format!(
            "抖音草稿已保存（ID: {}）。抖音发布接口要求封面与素材，请在创作者中心确认后点发布",
            returned_id
        )
    } else {
        format!("抖音草稿已保存！草稿 ID: {}", returned_id)
    };

    delay_return(
        start,
        PublishResult::success(returned_id, final_edit_url, message, operation, false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_detection() {
        assert!(is_login_url("https://sso.douyin.com/login/"));
        assert!(is_login_url("https://creator.douyin.com/login"));
        assert!(!is_login_url("https://creator.douyin.com/creator-micro/content/post/article"));
    }

    #[test]
    fn logged_in_url_detection() {
        assert!(is_logged_in_url(
            "https://creator.douyin.com/creator-micro/content/post/article"
        ));
        assert!(is_logged_in_url("https://creator.douyin.com/creator-micro/home"));
        // 未登录时的扫码登录页同样挂在 creator.douyin.com 上，不能算已登录。
        assert!(!is_logged_in_url("https://creator.douyin.com/"));
        assert!(!is_logged_in_url("https://creator.douyin.com/login"));
    }

    #[test]
    fn editor_page_detection() {
        assert!(is_editor_page(
            "https://creator.douyin.com/creator-micro/content/post/article"
        ));
        assert!(!is_editor_page("https://creator.douyin.com/creator-micro/home"));
    }

    #[test]
    fn creation_id_shape() {
        let id = create_creation_id();
        assert!(id.len() >= 21, "creation_id 太短: {}", id);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn hash_status_roundtrip() {
        let body = "DONE:{\"creationId\":\"abc123\",\"editUrl\":\"https://x/y\"}";
        let encoded = "DONE%3A%7B%22creationId%22%3A%22abc123%22%2C%22editUrl%22%3A%22https%3A%2F%2Fx%2Fy%22%7D";
        let url = format!("https://creator.douyin.com/x#openpub_res={}", encoded);
        let decoded = status_from_url(&url).expect("应能从 hash 解出状态");
        assert_eq!(decoded, body);
        match terminal_result(&decoded) {
            Some(DouyinTerminal::Done { creation_id, .. }) => assert_eq!(creation_id, "abc123"),
            _ => panic!("应解析为 Done"),
        }
    }

    #[test]
    fn auth_status_detection() {
        assert!(matches!(
            terminal_result("AUTH:需要验证"),
            Some(DouyinTerminal::Auth(_))
        ));
        assert!(matches!(
            terminal_result("FAIL:boom"),
            Some(DouyinTerminal::Fail(_))
        ));
        assert!(terminal_result("RUNNING").is_none());
    }
}