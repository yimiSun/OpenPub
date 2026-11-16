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
use crate::platforms::shared::{markdown, truncate};
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

/// 判定登录成功的连续命中次数。
///
/// 抖音登录成功后会先跳到 `creator-micro` 的某个路由，再被风控 / 埋点二次
/// 跳转；「首次命中即成功」会让还没扫完码的用户被判定为已登录，窗口随即
/// 关闭，随后 `draft` 接口必回 `status_code=8`。连续 3 次（间隔 500ms，
/// 共 1.5s）都停在已登录路由才算真正登录完成。
const LOGIN_STABLE_PROBES: u32 = 3;

/// 隐藏窗口静默探测登录态的时长，与头条通道保持一致。
const SILENT_PROBE_MS: u64 = 8_000;

/// 检测到登录后停留多久再关窗。
///
/// 不留这段时间，窗口会在用户扫码成功的瞬间消失，用户根本来不及确认
/// 「到底登上了没有」——这正是此前「窗口秒关、状态说不清」的体验来源。
const LOGIN_BANNER_LINGER_MS: u64 = 800;

/// 登录状态悬浮条。幂等：已存在就复用，重复注入不会叠出多条。
///
/// `pointer-events: none` 是刻意加的：这条栏固定在页面顶部，若参与命中测试
/// 会挡住抖音自己页面顶部的导航与按钮，用户点不动页面只会更困惑。
const LOGIN_BANNER_SCRIPT: &str = r##"(function () {
  var BANNER_ID = "__openpub_login_banner";
  var WAIT_TEXT = "【快发布】尚未检测到抖音登录态，请在本窗口完成扫码登录；登录成功后窗口会自动关闭。";
  var bar = document.getElementById(BANNER_ID);
  if (!bar) {
    bar = document.createElement("div");
    bar.id = BANNER_ID;
    bar.style.cssText = "position:fixed;top:0;left:0;right:0;z-index:2147483647;"
      + "padding:10px 16px;font:14px/1.6 system-ui;color:#fff;"
      + "background:#b45309;box-shadow:0 2px 8px rgba(0,0,0,.3);pointer-events:none";
    document.documentElement.appendChild(bar);
  }
  if (!bar.textContent) { bar.textContent = WAIT_TEXT; }
  window.__openpubSetLoginBanner = function (text, ok) {
    bar.textContent = text;
    bar.style.background = ok ? "#15803d" : "#b45309";
  };
})();"##;

/// 悬浮条切到「等待登录」（橙色）。
const BANNER_WAIT: &str = "try { window.__openpubSetLoginBanner && window.__openpubSetLoginBanner('【快发布】尚未检测到抖音登录态，请在本窗口完成扫码登录；登录成功后窗口会自动关闭。', false); } catch (e) {}";

/// 悬浮条切到「已登录，正在准备发布」（绿色）。
const BANNER_READY: &str = "try { window.__openpubSetLoginBanner && window.__openpubSetLoginBanner('已检测到抖音登录态，正在准备发布…', true); } catch (e) {}";

/// 悬浮条切到「登录成功，正在关闭窗口」（绿色）。
const BANNER_CLOSING: &str = "try { window.__openpubSetLoginBanner && window.__openpubSetLoginBanner('登录成功，正在关闭窗口…', true); } catch (e) {}";

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
            if !douyin_logged_in(&existing) {
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
            let mut streak = 0u32;
            loop {
                if app.get_webview_window(&label).is_none() {
                    return Err("抖音登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
                }
                // 隐藏窗口的定时器会被 WebView2 节流，但 Cookie 读取是 Rust 侧
                // 同步调用，不受影响，所以静默探测用 Cookie 判据是安全的。
                if douyin_logged_in(&window) {
                    streak += 1;
                    if streak >= LOGIN_STABLE_PROBES {
                        break;
                    }
                } else {
                    streak = 0;
                }
                if Instant::now() >= probe_deadline {
                    let url = window.url().map(|u| u.to_string()).unwrap_or_default();
                    let (logged_in, names) = auth_cookie_snapshot(&window);
                    eprintln!(
                        "[douyin] 静默探测未确认登录（logged_in={} keys=[{}]），显示窗口等待用户处理，当前页面: {}",
                        logged_in,
                        names.join(","),
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
    let mut streak = 0u32;
    while Instant::now() < deadline {
        if app.get_webview_window(&label).is_none() {
            return Err("抖音登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
        }

        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url != last_url {
            eprintln!("[douyin] 登录窗口页面变化: {}", url);
            last_url = url.clone();
        }

        // 唯一判据是 Cookie，不看 URL（SPA 未登录时 URL 同样像已登录）。
        let logged_in = douyin_logged_in(&window);
        // 悬浮条每轮都「重注入 + 重设状态」：脚本本身幂等，重复注入不会叠条；
        // 而只重注入不重设状态会让 SPA 清掉 DOM 后新建的条退回橙色默认文案，
        // 与真实登录态矛盾——这正是要避免的「用户判断不了状态」。
        refresh_login_banner(&window, if logged_in { BANNER_READY } else { BANNER_WAIT });

        if logged_in {
            streak += 1;
            if streak >= LOGIN_STABLE_PROBES {
                let (_, names) = auth_cookie_snapshot(&window);
                eprintln!(
                    "[douyin] Cookie 判定已登录 keys=[{}]，准备进入长文编辑页",
                    names.join(",")
                );
                ensure_editor_page(&window).await?;
                return Ok(window);
            }
        } else {
            if streak > 0 {
                eprintln!("[douyin] 登录态判定回退，重新累计（logged_in=false）");
            }
            streak = 0;
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    let (logged_in, names) = auth_cookie_snapshot(&window);
    Err(format!(
        "等待抖音登录超时（Cookie 里未出现 passport_assist_user，keys=[{}]，logged_in={}），请在通道窗口里完成扫码登录后重试",
        names.join(","),
        logged_in
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
    // 与 `ensure_login_window` 同一口径：连续命中才算登录成功，避免用户
    // 刚被跳到 `creator-micro` 就被判成已登录、窗口立刻关掉。
    let mut streak = 0u32;

    loop {
        if Instant::now() >= deadline {
            // 与思否同理：抖音登录会跳 `sso.douyin.com` 扫码页，超时关窗会把
            // 用户刚完成的扫码结果一起丢掉，只回传失败释放前端锁即可。
            // 这是「非确定失败」：窗口仍保留，用户可能正在扫码，前端不该据此
            // 把状态降成「未登录」。
            crate::login::emit_failed(
                app,
                PLATFORM,
                "等待抖音登录超时，窗口已保留；完成登录后请重新同步",
            );
            return;
        }

        let Some(window) = app.get_webview_window(&label) else {
            // 窗口被用户关掉且没检测到登录态，是确定失败，前端可以据此标「未登录」。
            crate::login::emit_auth_invalid(app, PLATFORM, "登录窗口已关闭，未检测到抖音登录态");
            return;
        };

        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url != last_url {
            eprintln!("[douyin] 登录页变化: {}", url);
            last_url = url.clone();
        }

        // 判据只看 Cookie，不看 URL：SPA 未登录时 URL 也会像已登录。
        let logged_in = douyin_logged_in(&window);
        refresh_login_banner(&window, if logged_in { BANNER_READY } else { BANNER_WAIT });

        if logged_in {
            streak += 1;
            if streak >= LOGIN_STABLE_PROBES {
                let (_, names) = auth_cookie_snapshot(&window);
                eprintln!(
                    "[douyin] Cookie 判定登录成功 keys=[{}]，{}ms 后关窗",
                    names.join(","),
                    LOGIN_BANNER_LINGER_MS
                );
                crate::login::emit_success(app, PLATFORM);
                // 先把悬浮条切绿并停留一段时间：窗口在扫码成功瞬间消失，
                // 用户无法确认自己到底登上了没有。停留后再关，反馈才算完整。
                let _ = window.eval(BANNER_CLOSING);
                sleep(Duration::from_millis(LOGIN_BANNER_LINGER_MS)).await;
                let _ = window.close();
                return;
            }
        } else {
            if streak > 0 {
                eprintln!("[douyin] 登录态判定回退，重新累计（logged_in=false）");
            }
            streak = 0;
        }

        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }
}

/// 与原始插件 `checkAuth` 同一判据：`.douyin.com` 上的 `passport_assist_user`。
///
/// 返回 `(是否已登录, 读到的 Cookie 键名)`。**值必须非空**：匿名访客也可能
/// 拿到同名的空值 Cookie，只看键名存在会误判成已登录。
///
/// 读取走 WebView2 底层 Cookie 容器（`cookies_for_url`），包含 `HttpOnly`，
/// 不受页面内 `document.cookie` 的限制——抖音的登录凭据正是 HttpOnly 的。
fn auth_cookie_snapshot(window: &tauri::WebviewWindow) -> (bool, Vec<String>) {
    let mut names: Vec<String> = Vec::new();
    let mut present = false;
    for url in ["https://creator.douyin.com/", "https://www.douyin.com/"] {
        if let Ok(parsed) = tauri::Url::parse(url) {
            if let Ok(cookies) = window.cookies_for_url(parsed) {
                for cookie in cookies.iter() {
                    let name = cookie.name().to_string();
                    if name == "passport_assist_user" && !cookie.value().trim().is_empty() {
                        present = true;
                    }
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    (present, names)
}

/// 抖音是否真的已登录。**唯一判据**：Cookie 里 `passport_assist_user` 值非空。
///
/// 这里刻意不看 URL。抖音创作者中心是 SPA，未登录时同样会渲染出
/// `/creator-micro/...` 路由骨架，URL 判据会给出「已登录」的假信号，
/// 于是窗口 1.5 秒内自动关闭，随后 draft 接口必回 `status_code=8`。
/// 用户看到的是「窗口秒关 + 提示已登录 + 发布说没登录」，三句话互相矛盾。
fn douyin_logged_in(window: &tauri::WebviewWindow) -> bool {
    auth_cookie_snapshot(window).0
}

/// 刷新登录状态悬浮条。
///
/// 每轮判定都重注入一次 `LOGIN_BANNER_SCRIPT`：它是幂等的，同时能覆盖
/// 「整页重新加载」与「SPA 路由切换把 DOM 清掉」两种丢失场景。
fn refresh_login_banner(window: &tauri::WebviewWindow, state: &str) {
    let _ = window.eval(LOGIN_BANNER_SCRIPT);
    let _ = window.eval(state);
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

  // 诊断信息单独走一条 title 通道（`DIAG:`）：Rust 侧只打日志、不当终态，
  // 避免把一大段响应体塞进用户可见的错误提示里。
  //
  // 返回 Promise 且故意留 400ms 间隔：紧随其后的 `setStatus` 会立刻覆盖标题，
  // Rust 侧 250ms 轮询若没赶在覆盖前读一次，这条诊断就丢了。
  function diag(text) {
    try { document.title = PREFIX + "DIAG:" + String(text).slice(0, 1200); } catch (e) {}
    return new Promise(function (resolve) { setTimeout(resolve, 400); });
  }

  // `document.cookie` 读不到 `HttpOnly` 项，读不到就返回空串，由调用方跳过该头。
  function readCookie(name) {
    try {
      var parts = (document.cookie || "").split(";");
      for (var i = 0; i < parts.length; i++) {
        var pair = parts[i];
        var eq = pair.indexOf("=");
        if (eq < 0) { continue; }
        if (pair.slice(0, eq).trim() === name) { return pair.slice(eq + 1).trim(); }
      }
    } catch (e) {}
    return "";
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

      // 抖音网页端的 draft 接口本身不校验 CSRF：原始插件 `executeInDouyinTab`
      // 同样只带 `Content-Type` + `credentials: "include"`，不带任何 csrf 头。
      // 但如果 Cookie 里确实存在 `passport_csrf_token`，顺手带上没有副作用，
      // 也能覆盖「平台后续收紧校验」的情况。`document.cookie` 读不到 HttpOnly
      // 项时 `readCookie` 返回空串，此处直接跳过，绝不臆造头。
      var headers = { "Content-Type": "application/json" };
      var csrf = readCookie("passport_csrf_token");
      if (!csrf) { csrf = readCookie("passport_csrf_token_default"); }
      if (csrf) { headers["x-tt-passport-csrf-token"] = csrf; }

      // 不手动拼 Cookie：交给 WebView2 按请求 URL 自动带上同源 Cookie。
      // 手动拼会覆盖浏览器行为，反而丢掉 `HttpOnly` 项。
      var response = await fetch(P.draftUrl, {
        method: "POST",
        credentials: "include",
        headers: headers,
        body: JSON.stringify(payload)
      });

      // 先读完响应体：无论成功失败都留一份诊断日志，`status_code=8` 时才有据可查。
      var text = await response.text();
      var decision = response.headers.get("x-tt-verify-passport-decision");
      var gateway = response.headers.get("bd_passport_security_gateway");
      var logId = response.headers.get("x-tt-logid") || "";
      await diag("draft HTTP " + response.status
        + " decision=" + (decision || "-")
        + " gateway=" + (gateway || "-")
        + " logid=" + (logId || "-")
        + " csrf=" + (csrf ? "present" : "absent")
        + " body=" + text.slice(0, 800));

      // 抖音风控不走 HTTP 状态码，而是回这两个响应头让前端自行跳验证页。
      if (decision || gateway === "1") {
        setStatus("AUTH:抖音要求账号安全验证（" + (decision || "security_gateway") + "），请在该窗口完成验证后重试");
        return;
      }

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
        // `status_code=8` 语义就是「未登录」：走 AUTH 通道让 Rust 侧显窗等用户
        // 扫码并自动重发，而不是混在普通 FAIL 文本里只报一句失败。
        if (json.status_code === 8) {
          // 响应体里若出现二次验证字样，说明不是单纯的登录态缺失，而是触发了
          // 风控（滑块 / 安全中心），把这层信息透出来，用户才知道该做什么。
          var verify = /verify_center|security_check|verify|captcha/i.test(text)
            ? "（响应提示二次验证，请在该窗口内完成验证）"
            : "";
          setStatus("AUTH:抖音登录态已失效（status_code=8），请在窗口内重新登录" + verify);
          return;
        }
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
            // `DIAG:` 是脚本写的一次性诊断通道（响应状态码 / 风控头 / 响应体），
            // 只打日志，不参与终态判定，也不给用户看。
            if let Some(rest) = title
                .strip_prefix(STATUS_PREFIX)
                .and_then(|rest| rest.strip_prefix("DIAG:"))
            {
                eprintln!("[douyin] 诊断 {}", rest);
            } else {
                eprintln!("[douyin] 窗口标题变化: {}", title);
            }
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

/// 收到未登录 / 需验证的终态后，把窗口交给用户：显示并聚焦，等登录稳定、
/// 回到长文编辑页，之后由调用方重发一次请求。
async fn recover_login(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<(), String> {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();

    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + Duration::from_millis(LOGIN_CHECK_TIMEOUT_MS);
    let mut last_url = String::new();
    let mut streak = 0u32;

    while Instant::now() < deadline {
        if app.get_webview_window(&label).is_none() {
            return Err("抖音登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
        }

        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url != last_url {
            eprintln!("[douyin] 等待重新登录，页面变化: {}", url);
            last_url = url.clone();
        }

        // 与 `ensure_login_window` 同一判据：只看 Cookie，不看 URL。
        let logged_in = douyin_logged_in(&window);
        refresh_login_banner(&window, if logged_in { BANNER_READY } else { BANNER_WAIT });

        if logged_in {
            streak += 1;
            if streak >= LOGIN_STABLE_PROBES {
                return ensure_editor_page(window).await;
            }
        } else {
            streak = 0;
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    let (logged_in, names) = auth_cookie_snapshot(window);
    Err(format!(
        "等待抖音重新登录超时（Cookie 里未出现 passport_assist_user，keys=[{}]，logged_in={}）",
        names.join(","),
        logged_in
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

    // 抖音长文编辑器吃 HTML，传 Markdown 原文会把 `#`、`-`、`|` 直接显示出来。
    let long_article = markdown::to_html_content(&request.content);
    eprintln!(
        "[douyin] long_article 前 100 字符: {}",
        truncate::truncate_chars(&long_article, 100)
    );

    let params = json!({
        "draftUrl": DRAFT_URL,
        "title": request.title.trim(),
        "markdown": long_article,
        "creationId": creation_id,
        "initTimestamp": timestamp,
        "editUrl": target_edit_url,
    });
    let script = build_draft_script(&params.to_string());

    // 注入前最后一道闸门：窗口仍可能被抖音风控异步顶到扫码 / 验证页，
    // 在那种页面上执行 `fetch` 是跨域请求，只会拿到 CORS 错误。
    let url_before_inject = window.url().map(|u| u.to_string()).unwrap_or_default();
    eprintln!("[douyin] 注入前页面: {}", url_before_inject);

    // Cookie 快照：`status_code=8` 到底是「没登录」还是「登录了但会话没带上」，
    // 靠这一行就能分清。`passport_assist_user` 是原始插件 `checkAuth` 的判据。
    let (has_auth_cookie, cookie_names) = auth_cookie_snapshot(&window);
    eprintln!(
        "[douyin] 通道窗口 Cookie 共 {} 项 keys=[{}] passport_assist_user={}",
        cookie_names.len(),
        cookie_names.join(","),
        has_auth_cookie
    );
    if !has_auth_cookie {
        eprintln!(
            "[douyin] 警告：Cookie 里没有 passport_assist_user，draft 请求大概率被判未登录（status_code=8）"
        );
    }

    if !is_editor_page(&url_before_inject) {
        // 一律把窗口留给用户，绝不关窗：没停在编辑页时用户唯一的补救手段
        // 就是在这个窗口里登录 / 过验证，关掉等于把补救入口一起收走。
        let _ = window.show();
        let _ = window.set_focus();
        refresh_login_banner(&window, BANNER_WAIT);
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

    let mut fetched = run_fetch_phase(&window, &script).await;

    // 抖音把「未登录」编码成 `status_code=8`（脚本已归到 AUTH 通道）。此时不要
    // 直接报错收场：把窗口弹给用户扫码，登录稳定后自动重发一次，用户不需要
    // 重新点同步。只重试一次，第二次仍失败就如实报错。
    if let Err(ChannelError::Auth(message)) = &fetched {
        let reason = message.clone();
        eprintln!("[douyin] 首次请求被判未登录（{}），等待用户重新登录后自动重试", reason);

        match recover_login(app, &window).await {
            Ok(()) => {
                eprintln!("[douyin] 重新登录完成，重发草稿请求");
                fetched = run_fetch_phase(&window, &script).await;
            }
            Err(error) => {
                let _ = window.show();
                let _ = window.set_focus();
                return delay_return(
                    start,
                    PublishResult::failure(
                        FailureClass::AuthInvalid,
                        format!("抖音要求账号安全验证：{}（{}）", reason, error),
                        operation,
                        false,
                    ),
                );
            }
        }
    }

    let (returned_id, returned_edit_url) = match fetched {
        Ok(value) => value,
        Err(ChannelError::Auth(message)) => {
            // 同上：认证类失败一律保留窗口，让用户就地补救（扫码或过安全验证）。
            let _ = window.show();
            let _ = window.set_focus();
            refresh_login_banner(&window, BANNER_WAIT);
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

    #[test]
    fn script_routes_status_code_8_to_auth_channel() {
        let script = build_draft_script("{}");
        assert!(script.contains("json.status_code === 8"), "{}", script);
        assert!(
            script.contains("AUTH:抖音登录态已失效"),
            "脚本必须把未登录归到 AUTH 通道"
        );
        // 其它非零状态码仍是普通失败，不能被误当成需要重新登录
        assert!(script.contains("FAIL:"), "{}", script);
    }

    #[test]
    fn script_embeds_html_long_article() {
        let long_article = markdown::to_html_content("## 小标题\n\n- a\n- b");
        assert!(long_article.contains("<h2>"), "{}", long_article);
        assert!(long_article.contains("<ul>"), "{}", long_article);

        let params = json!({
            "draftUrl": DRAFT_URL,
            "title": "标题",
            "markdown": long_article,
            "creationId": "cid",
            "initTimestamp": 1u64,
            "editUrl": EDITOR_URL,
        });
        let script = build_draft_script(&params.to_string());
        assert!(!script.contains("__OPENPUB_PARAMS__"));
        assert!(script.contains("long_article:"), "{}", script);
        assert!(script.contains("&lt;h2&gt;") || script.contains("<h2>"), "{}", script);
        assert!(!script.contains("## 小标题"), "脚本里不应残留 Markdown 原文");
    }

    #[test]
    fn script_sends_same_origin_credentials_without_manual_cookie() {
        let script = build_draft_script("{}");
        // 同源 Cookie 必须交给 WebView2 自动带上（含 HttpOnly），不能手动拼。
        assert!(
            script.contains("credentials: \"include\""),
            "脚本必须显式 credentials:include"
        );
        assert!(
            !script.contains("\"Cookie\""),
            "脚本不得手动设置 Cookie 头，否则会丢掉 HttpOnly 项"
        );
        // CSRF 头只在 Cookie 里真的有时才带，读不到就不臆造。
        assert!(
            script.contains("passport_csrf_token"),
            "脚本应从 Cookie 里尝试取 passport_csrf_token"
        );
        assert!(
            script.contains("if (csrf) { headers[\"x-tt-passport-csrf-token\"] = csrf; }"),
            "CSRF 头必须有条件地添加"
        );
    }

    #[test]
    fn script_emits_diagnostics_on_non_zero_status() {
        let script = build_draft_script("{}");
        // 诊断通道必须在读完响应体之后、判定终态之前触发，且不参与终态判定。
        assert!(script.contains("await diag("), "{}", script);
        assert!(script.contains("\"DIAG:\""), "{}", script);
        let diag_at = script.find("await diag(").expect("diag");
        let auth_at = script.find("AUTH:抖音登录态已失效").expect("auth");
        assert!(diag_at < auth_at, "诊断必须先于终态上报");
        assert!(
            script.contains("x-tt-verify-passport-decision"),
            "诊断要包含风控判定头"
        );
    }

    #[test]
    fn diag_title_is_not_treated_as_terminal() {
        // `DIAG:` 不能被 terminal_result 认成终态，否则诊断会提前结束轮询。
        assert!(terminal_result("DIAG:draft HTTP 200").is_none());
    }

    #[test]
    fn login_banner_is_idempotent_and_click_through() {
        assert!(LOGIN_BANNER_SCRIPT.contains("__openpub_login_banner"));
        // 幂等：已存在就复用，每 500ms 重注入才不会叠出多条。
        assert!(LOGIN_BANNER_SCRIPT.contains("if (!bar)"));
        assert!(LOGIN_BANNER_SCRIPT.contains("getElementById"));
        // 固定栏不能参与命中测试，否则会挡住抖音自己页面顶部的按钮。
        assert!(LOGIN_BANNER_SCRIPT.contains("pointer-events:none"));
        assert!(LOGIN_BANNER_SCRIPT.contains("__openpubSetLoginBanner"));
        // 必须挂在 documentElement 上，挂在 body 上会被部分页面的样式裁掉。
        assert!(LOGIN_BANNER_SCRIPT.contains("document.documentElement.appendChild"));
    }

    #[test]
    fn banner_states_cover_wait_and_success() {
        // 第二参数是颜色开关：false=橙（等待），true=绿（已登录）。
        assert!(BANNER_WAIT.trim_end().ends_with(", false); } catch (e) {}"));
        assert!(BANNER_READY.contains(", true);"));
        assert!(BANNER_CLOSING.contains("登录成功，正在关闭窗口"));
        // 登录成功必须停留一段可感知的时间，否则窗口秒关、用户无从确认。
        assert_eq!(LOGIN_BANNER_LINGER_MS, 800);
    }

    #[test]
    fn login_stability_requires_three_probes() {
        assert_eq!(LOGIN_STABLE_PROBES, 3);
        assert_eq!(LOGIN_CHECK_INTERVAL_MS, 500);
    }
}