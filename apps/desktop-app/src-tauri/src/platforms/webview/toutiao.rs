//! 今日头条通道适配器（页面上下文内 fetch 建草稿 / 发布）。
//!
//! 头条的发布接口 `/mp/agw/article/publish` 带页面级风控：Rust 侧用 `reqwest`
//! 拼齐全部字段（含 `source=29`、`article_ad_type`、`extra`）仍会稳定返回
//! `7050 参数校验失败`。原始插件 `index.mjs` 的 `publishViaEditor` 是通过
//! `chrome.tabs.executeScript` 在**头条编辑页自己的上下文**里发 `fetch`，
//! 浏览器的页面环境特征（`n_mh` / `s_v_web_id` / 页面 SDK 注入的 CSRF 标识）
//! 与请求天然一致，因此不会被判伪造。
//!
//! 这里照搬同一模型：
//! 1. `open_channel_window` 到 `https://mp.toutiao.com/profile_v4/graphic/publish`
//! 2. 等页面落到编辑页（未登录会跳 `/auth/page/login`），注入脚本：
//!    a. 先 `GET` 广告权限接口，决定 `article_ad_type` 传 `"2"` 还是 `"3"`
//!    b. 用原始插件的完整字段集 `POST` 建草稿
//!    c. 勾选发布时再 `POST` 一次（`save=1`、`pgc_id` 换成刚拿到的草稿 ID）
//!    d. 结果写进 `document.title`
//! 3. Rust 侧轮询 `window.title()` 取回结果，`window.close()` 收尾。
//!
//! 表单在 Rust 侧按原始插件 `buildArticleBody` 逐字段构造，只把
//! `article_ad_type` 与发布步的 `pgc_id` 留成占位符，交给页面脚本回填。

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, open_channel_window, open_channel_window_with_visibility};
use crate::platforms::shared::markdown;
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "toutiao";

/// 头条图文编辑页。脚本必须在它自己的页面上下文里执行。
pub const EDITOR_URL: &str = "https://mp.toutiao.com/profile_v4/graphic/publish";
/// 发布 / 存草稿接口，与原始插件 `publishViaEditor` 完全一致。
const PUBLISH_URL: &str = "https://mp.toutiao.com/mp/agw/article/publish?source=mp&type=article&aid=1231";
/// 广告权限查询接口，原始插件 `getArticleAdType` 在保存前必先请求。
const AD_TYPE_URL: &str =
    "https://mp.toutiao.com/mp/agw/article/new?article_type=0&format=json&compat=1&column_no=";

pub const STATUS_PREFIX: &str = "OPENPUB: ";

const SCRIPT_TIMEOUT_MS: u64 = 45_000;
const POLL_INTERVAL_MS: u64 = 250;
const WINDOW_WAIT_MS: u64 = 3_000;
const REINJECT_AFTER_MS: u64 = 3_000;
const MAX_INJECT_ATTEMPTS: u32 = 4;

/// 结果 hash 的键名。脚本侧写的是 `#openpub_res=<encodeURIComponent(state)>`。
const RESULT_HASH_KEY: &str = "openpub_res=";

/// 每次新的发布尝试注入前先清掉上一轮的会话标记。
///
/// 脚本用 `sessionStorage` 记住「已经发起过」和「上一轮终态」，防止重注入时
/// 二次建草稿；但若不清，新的发布尝试会把上一轮的 `FAIL` 直接回放成这一轮的结果。
const CLEAR_SESSION_SCRIPT: &str = "try { sessionStorage.removeItem('__openpub_tt_state__'); sessionStorage.removeItem('__openpub_tt_started__'); } catch (e) {}";

const LOGIN_CHECK_TIMEOUT_MS: u64 = 300_000;
const LOGIN_CHECK_INTERVAL_MS: u64 = 500;

/// 隐藏窗口静默探测登录态的时长。头条编辑页正常加载落在 1~3 秒，
/// 给 8 秒余量：超时仍未落到站内页，说明 Cookie 失效或需要验证，此时才显示窗口。
const SILENT_PROBE_MS: u64 = 8_000;

/// 判定「这个 URL 处于登录 / 风控拦截流程中，绝不能当成已登录」。
///
/// 头条的拦截页远不止登录页一种，实测至少三类，且**全部挂在 `mp.toutiao.com`
/// 上**（不是 `sso.toutiao.com` 子域）：
/// - `/auth/page/login/`：未登录
/// - `/auth/page/bind_mobile_index/`：账号被要求绑定手机号（风控）
/// - `/auth/page/verify/` 等其它 `/auth/page/*` 子页：二次校验
///
/// 这些页面的 `redirect_url` 查询串里都带着 `profile_v4/graphic/publish`，
/// 所以「URL 含 `/profile_v4` 就算已登录」是错的——必须**先**把整个
/// `/auth/page` 前缀连同 `sso.toutiao.com`、`/bind_mobile` 一起排除。
pub fn is_login_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("sso.toutiao.com")
        || url.contains("/auth/page")
        || url.contains("/bind_mobile")
        || url.contains("passport.")
        || url.contains("/login")
}

pub fn is_logged_in_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("mp.toutiao.com")
        && !is_login_url(&url)
        && (url.contains("/profile_v4") || url.contains("/graphic/publish"))
}

/// 严格判定「当前窗口确实停在头条图文编辑页」。
///
/// 注入脚本**只能**在这个页面上执行：换成 `auth/page/*` 或 `sso.toutiao.com`
/// 时 `fetch` 目标是跨域，必报 CORS；换成 `/profile_v4/index` 时页面级风控
/// 上下文不成立，接口会回 `7050`。两条都会表现成「保存失败」，但根因不同，
/// 因此这里单独给一个谓词，让调用方先确认页面再注入。
pub fn is_editor_page(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("mp.toutiao.com")
        && !is_login_url(&url)
        && url.contains("/profile_v4/graphic/publish")
}

async fn ensure_login_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let label = channel_window_label(PLATFORM);

    // 已有窗口一律复用，**绝不关窗重开**。
    //
    // 之前按 URL 分类决策：停在登录 / 绑定手机页就等，其它页面就关掉重开。
    // 这条规则会踩到第三方授权域：点微信登录后窗口跳到 `open.weixin.qq.com`，
    // 点 QQ 登录跳到 `graphic.qq.com` —— 两者既不是头条登录页、也不是站内，
    // 按旧规则会被归为「异常页」直接关掉，用户刚扫完码授权就被踢出窗口，
    // 表现为「点微信登录、扫完码窗口没了、还是没登上」。
    //
    // 因此这里只做一件事：把已有窗口拉到前台，然后等它自己跳回站内编辑页。
    // 用户中途手动关窗会被下面的存在性检查发现并报错。
    //
    // 唯一例外是「已经确认登录」的情况：此时把窗口 show/focus 只会让用户每次
    // 同步都被窗口闪一下，完全没必要。只有停在登录 / 绑定 / 第三方授权页时
    // 才需要用户介入，这时才抢焦点。
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
            // 首次静默探测：先隐藏建窗，若 WebView2 profile 里的 Cookie 仍然有效，
            // 页面会在几秒内自己落到站内页，全程不打扰用户；只有落不了地（Cookie
            // 过期 / 需要验证）才显示窗口让用户就地登录。
            let window = open_channel_window_with_visibility(app, PLATFORM, EDITOR_URL, false)?;
            let probe_deadline = Instant::now() + Duration::from_millis(SILENT_PROBE_MS);
            loop {
                if app.get_webview_window(&label).is_none() {
                    return Err(
                        "今日头条登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string(),
                    );
                }
                let url = window.url().map(|u| u.to_string()).unwrap_or_default();
                if is_logged_in_url(&url) {
                    break;
                }
                if Instant::now() >= probe_deadline {
                    eprintln!(
                        "[toutiao] 静默探测未确认登录，显示窗口等待用户处理，当前页面: {}",
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
            return Err("今日头条登录窗口已关闭，请重新点击「打开平台页面」完成登录".to_string());
        }

        let url = window.url().map(|u| u.to_string()).unwrap_or_default();
        if url != last_url {
            eprintln!("[toutiao] 登录窗口页面变化: {}", url);
            last_url = url.clone();
        }
        if is_logged_in_url(&url) {
            ensure_editor_page(&window).await?;
            return Ok(window);
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    Err(format!(
        "等待今日头条登录超时，请在通道窗口里完成登录或账号验证后重试（当前页面: {}）",
        window.url().map(|u| u.to_string()).unwrap_or_default()
    ))
}

/// 确保窗口停在头条图文编辑页，否则导航过去并等待落地。
///
/// 登录成功后头条常常落在 `/profile_v4/index`，也可能因为风控被顶回
/// `/auth/page/bind_mobile_index`。这两种页面都不能注入脚本：
/// - 前者不是编辑页，页面级风控上下文不成立，接口回 `7050`；
/// - 后者是 `auth/page` 域下页面，`fetch` 到 `mp/agw/...` 是跨域，必报 CORS。
///
/// 因此这里用 `is_editor_page` 严格判定，落地后仍不是编辑页就返回错误，
/// 而不是硬着头皮注入。
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
        // 跳转后又被风控顶到登录 / 绑定手机页，再等也不会自己变成编辑页。
        if is_login_url(&url) {
            return Err(format!(
                "头条要求先完成登录或账号验证（当前页面: {}），请在通道窗口里处理后重试",
                url
            ));
        }
        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }

    let final_url = window.url().map(|u| u.to_string()).unwrap_or_default();
    Err(format!(
        "打开今日头条编辑页超时，当前页面: {}",
        final_url
    ))
}

async fn close_and_wait(app: &AppHandle) -> Result<(), String> {
    let label = channel_window_label(PLATFORM);
    if app.get_webview_window(&label).is_some() {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.get_webview_window(&label).is_some() {
            if Instant::now() >= deadline {
                return Err("今日头条通道窗口关闭超时，请稍后重试".to_string());
            }
            sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }
    Ok(())
}

/// 前端「打开平台页面」入口：拉起通道窗口到编辑页，登录态由 WebView2 的
/// 用户数据目录记住，发布时同一窗口直接复用，不写 DPAPI Cookie。
pub async fn open_login_channel(app: &AppHandle) -> Result<(), String> {
    close_and_wait(app).await?;
    open_channel_window(app, PLATFORM, EDITOR_URL)?;

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
            crate::login::emit_failed(
                app,
                PLATFORM,
                "今日头条登录超时，请重新点击「打开平台页面」扫码",
            );
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.close();
            }
            return;
        }

        let Some(window) = app.get_webview_window(&label) else {
            crate::login::emit_failed(app, PLATFORM, "登录窗口已关闭，未完成登录");
            return;
        };

        if let Ok(url) = window.url() {
            let url = url.to_string();
            if url != last_url {
                eprintln!("[toutiao] 登录页变化: {}", url);
                last_url = url.clone();
            }
            // 只用 `is_logged_in_url` 判定成功，**不能**用「访问过登录页后
            // 出现任意非登录 URL」这种捷径。点微信登录会跳到
            // `open.weixin.qq.com`，点 QQ 登录会跳到 `graphic.qq.com`，
            // 两者都不是头条登录页，旧逻辑会把它们当成「登录完成」立即
            // 上报成功并关窗，用户实际还没登上。
            if is_logged_in_url(&url) {
                crate::login::emit_success(app, PLATFORM);
                let _ = window.close();
                return;
            }
        }

        sleep(Duration::from_millis(LOGIN_CHECK_INTERVAL_MS)).await;
    }
}

fn split_status(body: &str) -> (String, String) {
    if let Some(colon_idx) = body.find(':') {
        let kind = body[..colon_idx].trim().to_string();
        let payload = body[colon_idx + 1..].trim().to_string();
        return (kind, payload);
    }
    // 无冒号是进度心跳（`OPENPUB: RUNNING`）。保留原样作为状态名，
    // 由调用方判定是否终态，而不是在这里统一抹成 `UNKNOWN`——
    // 抹平后既看不出脚本其实在正常跑，也会让日志失去排查价值。
    (body.trim().to_string(), String::new())
}

#[cfg(test)]
fn parse_status(title: &str) -> (String, String) {
    match title.strip_prefix(STATUS_PREFIX) {
        Some(rest) => split_status(rest),
        None => ("UNKNOWN".to_string(), title.to_string()),
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

/// 解 `encodeURIComponent` 产生的百分号编码。
///
/// 项目没有 `urlencoding` 依赖，为一个几十行的解码再引一个 crate 不值得，
/// 这里就地实现；`+` 保持原样（`encodeURIComponent` 把空格编码成 `%20`）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2])) {
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

/// 从窗口 URL 的 hash 取出脚本写的结果体（形如 `DONE:{...}` / `FAIL:...`）。
fn status_from_url(url: &str) -> Option<String> {
    let index = url.find(RESULT_HASH_KEY)?;
    let rest = &url[index + RESULT_HASH_KEY.len()..];
    let raw = rest.split('&').next().unwrap_or(rest);
    if raw.is_empty() {
        return None;
    }
    Some(percent_decode(raw))
}

/// 把终态状态体翻译成发布结果；非终态（`RUNNING` 等）返回 `None`。
fn terminal_result(body: &str) -> Option<Result<(String, Option<String>), String>> {
    let (kind, payload) = split_status(body);
    match kind.as_str() {
        "DONE" => Some(parse_done_payload(&payload)),
        "FAIL" => Some(Err(payload)),
        _ => None,
    }
}

fn parse_done_payload(payload: &str) -> Result<(String, Option<String>), String> {
    let parsed: serde_json::Value = serde_json::from_str(payload)
        .map_err(|_| format!("结果 JSON 解析失败，原始内容: {}", payload))?;
    let draft_id = parsed
        .get("draftId")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("结果缺少草稿 ID，原始内容: {}", payload))?;
    let published_id = parsed
        .get("publishedId")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    Ok((draft_id, published_id))
}

/// 同时读 URL hash 与窗口标题两个通道。
///
/// 返回 `(是否看到脚本写的状态, 终态结果)`。两个都要读：hash 抗标题抢占，
/// 标题抗页面清空 hash，任一命中即可。
fn read_status_channels(
    window: &tauri::WebviewWindow,
) -> (bool, Option<Result<(String, Option<String>), String>>) {
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

/// 按原始插件 `buildArticleBody` 的字段集与顺序构造表单。
///
/// `pgc_id` 与 `article_ad_type` 由调用方传入；脚本侧用占位符回填实际值，
/// 保证字段顺序与插件逐字一致（顺序错也可能被判参数校验失败）。
fn article_form(
    request: &PublishRequest,
    html: &str,
    pgc_id: &str,
    publish: bool,
    article_ad_type: &str,
) -> Vec<(&'static str, String)> {
    let mut extra = json!({
        "content_source": 100000000402_i64,
        "content_word_cnt": html.chars().count(),
        "is_multi_title": 0,
        "sub_titles": [],
        "gd_ext": {
            "entrance": "",
            "from_page": "publisher_mp",
            "enter_from": "PC",
            "device_platform": "mp",
            "is_message": 0,
        },
    });
    if publish && let Some(object) = extra.as_object_mut() {
        object.insert("tuwen_wtt_trans_flag".to_string(), json!("2"));
        object.insert("info_source".to_string(), json!({ "source_type": -1 }));
    }

    let title_id = format!(
        "{}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(6)
            .collect::<String>()
    );

    let mut form: Vec<(&'static str, String)> = vec![
        ("article_type", "0".to_string()),
        ("pgc_id", pgc_id.to_string()),
        ("source", "29".to_string()),
        ("extra", extra.to_string()),
        ("content", html.to_string()),
        ("title", request.title.clone()),
        (
            "search_creation_info",
            r#"{"searchTopOne":0,"abstract":"","clue_id":""}"#.to_string(),
        ),
        ("title_id", title_id),
    ];

    if publish {
        form.extend(vec![
            ("ic_uri_list", String::new()),
            ("appid_list", String::new()),
            ("stock_ids", String::new()),
            ("concern_list", String::new()),
        ]);
    }

    form.extend(vec![
        ("mp_editor_stat", "{}".to_string()),
        ("is_refute_rumor", "0".to_string()),
        ("save", if publish { "1" } else { "0" }.to_string()),
        ("entrance", if publish { "main" } else { "" }.to_string()),
        ("timer_status", "0".to_string()),
        ("timer_time", String::new()),
        ("educluecard", String::new()),
        ("draft_form_data", r#"{"coverType":3}"#.to_string()),
        ("pgc_feed_covers", "[]".to_string()),
        ("article_ad_type", article_ad_type.to_string()),
        ("is_fans_article", "0".to_string()),
        ("govern_forward", "0".to_string()),
        ("praise", "0".to_string()),
        ("disable_praise", "0".to_string()),
        ("tree_plan_article", "0".to_string()),
        ("star_order_id", String::new()),
        ("star_order_name", String::new()),
        ("activity_tag", "0".to_string()),
        ("trends_writing_tag", "0".to_string()),
        ("claim_exclusive", "0".to_string()),
    ]);

    form
}

fn build_fetch_script(params_json: &str) -> String {
    // 用 token 替换而非 `format!`：JS 里花括号极多，`format!` 需要把每一个
    // `{` / `}` 都写成 `{{` / `}}`，一旦漏改就会静默生成坏脚本。
    // 先替换前缀，再替换参数，避免参数内容里意外出现前缀 token 时被二次替换。
    let template = r##"(function () {
  var PREFIX = "__OPENPUB_PREFIX__";
  var P = __OPENPUB_PARAMS__;

  // 结果有两个通道：`document.title` 与 URL hash。
  //
  // 只靠 title 会漏采：头条编辑器自己会周期性把 `document.title` 抢回
  // 「头条号 - 你创作的」这类标题，而 WebView2 对非焦点窗口的定时器还有节流，
  // 于是我们写入的终态只在一小段空隙里可见。Rust 侧 250ms 轮询撞上那个空隙的
  // 概率很低——实测表现就是「草稿已经建好、draftId 已生成，Rust 却等到 45 秒
  // 超时」。URL hash 页面不会抢，是可靠的终态通道；title 保留为兜底。
  function isTerminal(text) {
    return text.indexOf("DONE:") === 0 || text.indexOf("FAIL:") === 0;
  }

  function writeHash(text) {
    try {
      var target = "#openpub_res=" + encodeURIComponent(text);
      if (location.hash !== target) { location.hash = target; }
    } catch (e) {}
  }

  function publish(text) {
    try { document.title = PREFIX + text; } catch (e) {}
    if (isTerminal(text)) { writeHash(text); }
  }

  // sessionStorage 在同一次发布尝试内跨页面跳转仍然有效（同标签页），用来防止
  // 「脚本丢失后重新注入 → 二次建草稿 / 二次发布」。Rust 侧每次新的发布尝试
  // 会先清掉这两个键（见 CLEAR_SESSION_SCRIPT），所以不会把上一轮的结果
  // 误当成这一轮。
  function load(key) {
    try { return sessionStorage.getItem(key); } catch (e) { return null; }
  }
  function store(key, value) {
    try { sessionStorage.setItem(key, value); } catch (e) {}
  }

  var previous = load("__openpub_tt_state__");
  if (previous && isTerminal(previous)) {
    // 上一轮已经跑完，只重放结果，绝不重跑。
    publish(previous);
    return;
  }
  if (load("__openpub_tt_started__")) {
    // 已经发起过但还没拿到终态：重跑会重复建草稿，因此只回放当前进度。
    publish(previous || "RUNNING");
    return;
  }
  if (window.__OPENPUB_TT_RUNNING__) { return; }
  window.__OPENPUB_TT_RUNNING__ = true;
  store("__openpub_tt_started__", "1");

  var state = "RUNNING";
  function setStatus(text) {
    state = text;
    store("__openpub_tt_state__", text);
    publish(text);
  }
  setInterval(function () { publish(state); }, 100);

  function applyReplacements(entries, replacements) {
    return entries.map(function (pair) {
      var key = pair[0];
      var value = pair[1];
      if (Object.prototype.hasOwnProperty.call(replacements, key)) {
        value = replacements[key];
      }
      return [key, value];
    });
  }

  function postForm(url, entries) {
    var body = new URLSearchParams();
    entries.forEach(function (pair) { body.append(pair[0], pair[1]); });
    return fetch(url, {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body: body.toString()
    }).then(function (resp) {
      return resp.text().then(function (text) {
        var json = null;
        try { json = JSON.parse(text); } catch (e) {}
        return { ok: resp.ok, status: resp.status, text: text, json: json };
      });
    });
  }

  function readErrNo(result) {
    if (!result.json) { return null; }
    if (result.json.err_no !== undefined) { return result.json.err_no; }
    if (result.json.code !== undefined) { return result.json.code; }
    return null;
  }

  function readMessage(result) {
    if (!result.json) { return ""; }
    return (result.json.message || result.json.msg || "");
  }

  function readPgcId(result) {
    if (!result.json || !result.json.data) { return null; }
    var id = result.json.data.pgc_id;
    if (id === undefined || id === null || String(id) === "") { return null; }
    return String(id);
  }

  async function fetchAdType() {
    try {
      var resp = await fetch(P.adTypeUrl, { credentials: "include" });
      if (!resp.ok) { return "2"; }
      var data = await resp.json();
      var no = (data && data.err_no !== undefined) ? data.err_no : (data && data.code);
      if (no !== 0 || !data.data) { return "2"; }
      var media = data.data.media_permissions || data.data.medias || {};
      if (media.can_toutiao_ad_select === false) { return "2"; }
      if (media.can_toutiao_ad_select === true) { return "3"; }
      if (typeof media.ads_permission === "number" && media.ads_permission > 0) { return "3"; }
      return "2";
    } catch (e) {
      return "2";
    }
  }

  async function run() {
    try {
      var adType = await fetchAdType();

      var draftForm = applyReplacements(P.draftForm, { article_ad_type: adType });
      var draft = await postForm(P.publishUrl, draftForm);

      if (!draft.ok) {
        setStatus("FAIL:保存草稿失败 HTTP " + draft.status + " " + draft.text.slice(0, 200));
        return;
      }
      if (!draft.json) {
        setStatus("FAIL:保存草稿响应不是 JSON: " + draft.text.slice(0, 200));
        return;
      }
      var draftErr = readErrNo(draft);
      if (draftErr !== 0) {
        setStatus("FAIL:" + (readMessage(draft) || "保存草稿失败") + " (err_no=" + draftErr + ") " + draft.text.slice(0, 200));
        return;
      }
      var draftId = readPgcId(draft);
      if (!draftId) {
        setStatus("FAIL:草稿响应缺少 pgc_id: " + draft.text.slice(0, 200));
        return;
      }

      var publishedId = null;
      if (P.publishForm) {
        var publishForm = applyReplacements(P.publishForm, { article_ad_type: adType, pgc_id: draftId });
        var pub = await postForm(P.publishUrl, publishForm);
        if (!pub.ok) {
          setStatus("FAIL:发布失败 HTTP " + pub.status + " " + pub.text.slice(0, 200));
          return;
        }
        if (!pub.json) {
          setStatus("FAIL:发布响应不是 JSON: " + pub.text.slice(0, 200));
          return;
        }
        var pubErr = readErrNo(pub);
        if (pubErr !== 0) {
          setStatus("FAIL:" + (readMessage(pub) || "发布失败") + " (err_no=" + pubErr + ") " + pub.text.slice(0, 200));
          return;
        }
        publishedId = readPgcId(pub) || draftId;
      }

      setStatus("DONE:" + JSON.stringify({ draftId: draftId, publishedId: publishedId }));
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
        .replace("__OPENPUB_PARAMS__", params_json)
}

async fn run_fetch_phase(
    window: &tauri::WebviewWindow,
    script: &str,
) -> Result<(String, Option<String>), String> {
    // 先清会话标记再注入：否则脚本会把上一轮的终态（尤其是 FAIL）直接回放。
    let _ = window.eval(CLEAR_SESSION_SCRIPT);

    if let Err(e) = window.eval(script) {
        return Err(format!("脚本注入失败：{}", e));
    }

    let mut attempts: u32 = 1;
    let mut last_status_at = Instant::now();
    let mut last_title = String::new();
    let deadline = Instant::now() + Duration::from_millis(SCRIPT_TIMEOUT_MS);

    while Instant::now() < deadline {
        // 两个通道都读：URL hash 抗标题抢占，窗口标题抗页面清空 hash。
        let (saw_status, terminal) = read_status_channels(window);
        if let Some(result) = terminal {
            return result;
        }
        if saw_status {
            // 还能看到脚本写的状态，说明脚本活着，不需要重注入。
            last_status_at = Instant::now();
        }

        let title = window.title().unwrap_or_default();
        if title != last_title {
            eprintln!("[toutiao] 窗口标题变化: {}", title);
            last_title = title;
        }

        // 头条编辑页是 SPA，加载中途的跳转会清掉已注入的脚本（`window` 上的
        // 幂等标记随之消失）。若一段时间看不到状态前缀，说明脚本已丢失，
        // 重新注入一次；同一页面内重复注入会被幂等标记挡掉，不会双发请求。
        if attempts < MAX_INJECT_ATTEMPTS
            && last_status_at.elapsed() >= Duration::from_millis(REINJECT_AFTER_MS)
        {
            let url = window.url().map(|u| u.to_string()).unwrap_or_default();
            eprintln!(
                "[toutiao] 第 {} 次重注入脚本，当前页面: {}",
                attempts + 1,
                url
            );
            let _ = window.eval(script);
            attempts += 1;
            last_status_at = Instant::now();
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }

    // 退出循环后再读一次：终态可能刚好落在最后一次轮询之后。
    let (_, terminal) = read_status_channels(window);
    if let Some(result) = terminal {
        return result;
    }

    let current_title = window.title().unwrap_or_default();
    let current_url = window.url().map(|u| u.to_string()).unwrap_or_default();
    Err(format!(
        "等待头条页面返回结果超时（{} 秒），当前窗口标题: {}，当前页面: {}",
        SCRIPT_TIMEOUT_MS / 1000,
        current_title,
        current_url
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

    if request.existing_published {
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::NotImplemented,
                "今日头条不支持修改已发布的文章，请先在头条后台将其转为草稿后再同步".to_string(),
                operation,
                false,
            ),
        );
    }

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

    // 建草稿这一步的 `pgc_id`：新建传 `"0"`，更新必须带旧草稿 ID。
    let draft_pgc_id = match operation {
        PublishOperation::Create => "0".to_string(),
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
                        "今日头条更新草稿时缺少草稿 ID，请重新同步一次".to_string(),
                        operation,
                        false,
                    ),
                )
            }
        },
    };

    let window = match ensure_login_window(app).await {
        Ok(window) => window,
        Err(error) => {
            return delay_return(
                start,
                PublishResult::failure(
                    FailureClass::AuthInvalid,
                    format!("今日头条登录失败：{}", error),
                    operation,
                    false,
                ),
            )
        }
    };

    let html = markdown::to_html_content(&request.content);

    // 广告权限在页面上下文里取，结果回填到 `article_ad_type` 占位符。
    let draft_form = article_form(request, &html, &draft_pgc_id, false, "__AD_TYPE__");
    let publish_form = if request.publish_status && !request.existing_published {
        Some(article_form(request, &html, "__PGC_ID__", true, "__AD_TYPE__"))
    } else {
        None
    };

    let params = json!({
        "publishUrl": PUBLISH_URL,
        "adTypeUrl": AD_TYPE_URL,
        "draftForm": draft_form,
        "publishForm": publish_form,
    });
    let script = build_fetch_script(&params.to_string());

    // 注入前最后一道闸门：`ensure_login_window` 返回后窗口仍可能被头条的
    // 风控异步顶到 `auth/page/*`。在那种页面上执行 `fetch` 是跨域请求，
    // 只会拿到 CORS 错误，不如在这里直接给出可读原因。
    let url_before_inject = window.url().map(|u| u.to_string()).unwrap_or_default();
    eprintln!("[toutiao] 注入前页面: {}", url_before_inject);
    if !is_editor_page(&url_before_inject) {
        // 停在登录 / 绑定手机页时**保留窗口**，让用户能就地处理；
        // 其它异常页面才关掉，避免留一个无用的窗口挡住用户。
        if !is_login_url(&url_before_inject) {
            let _ = window.close();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::AuthInvalid,
                format!(
                    "今日头条未停在图文编辑页，请在通道窗口里完成登录或账号验证后重试（当前页面: {}）",
                    url_before_inject
                ),
                operation,
                false,
            ),
        );
    }

    let fetched = run_fetch_phase(&window, &script).await;
    let (draft_id, published_id) = match fetched {
        Ok(value) => value,
        Err(error) => {
            // 失败时若窗口已被顶到登录 / 绑定页，保留窗口让用户就地处理，
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
                    format!("今日头条保存失败：{}", error),
                    operation,
                    false,
                ),
            );
        }
    };

    let _ = window.close();

    // 更新时服务端若返回了不同的 ID，说明它把这次更新当成新建了；静默采纳会
    // 污染同步表，必须报错让人到后台确认。
    if operation == PublishOperation::Update && draft_id != draft_pgc_id {
        return delay_return(
            start,
            PublishResult::failure(
                FailureClass::PlatformError,
                format!(
                    "草稿更新结果不一致：提交的 ID 为 {}，服务端返回 {}，请到头条后台确认",
                    draft_pgc_id, draft_id
                ),
                operation,
                false,
            ),
        );
    }

    let mut post_id = draft_id.clone();
    let mut is_published = request.existing_published;

    if let Some(published) = published_id {
        if published != draft_id {
            return delay_return(
                start,
                PublishResult::failure(
                    FailureClass::PlatformError,
                    "发布结果与草稿 ID 不一致，请在头条后台确认发布状态".to_string(),
                    operation,
                    false,
                ),
            );
        }
        post_id = published;
        is_published = true;
    }

    let edit_url = format!("{}?pgc_id={}", EDITOR_URL, draft_id);
    let message = if is_published {
        format!("已成功发布到今日头条！文章 ID: {}", post_id)
    } else {
        format!("今日头条草稿已保存！草稿 ID: {}", draft_id)
    };

    delay_return(
        start,
        PublishResult::success(post_id, edit_url, message, operation, is_published),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(publish_status: bool) -> PublishRequest {
        PublishRequest {
            title: "头条标题".to_string(),
            content: "## 正文\n\n第一行".to_string(),
            publish_status,
            ..Default::default()
        }
    }

    #[test]
    fn login_url_detection() {
        assert!(is_login_url(
            "https://mp.toutiao.com/auth/page/login/?redirect_url=https%3A%2F%2Fmp.toutiao.com%2Fprofile_v4%2Fgraphic%2Fpublish"
        ));
        assert!(is_login_url("https://sso.toutiao.com/login/"));
        assert!(!is_login_url(EDITOR_URL));
    }

    /// 回归：截图里那个「手机绑定」页挂在 `mp.toutiao.com/auth/page/bind_mobile_index`
    /// 上，`redirect_url` 里带着 `profile_v4`，旧的宽松判定把它当成已登录，
    /// 1 秒就报「登录成功」并关窗，随后在错误页面上注入脚本。
    #[test]
    fn bind_mobile_page_is_not_logged_in() {
        let bind_mobile = "https://mp.toutiao.com/auth/page/bind_mobile_index/?redirect_url=https%3A%2F%2Fmp.toutiao.com%2Fprofile_v4%2Fgraphic%2Fpublish&aid=1231";
        assert!(is_login_url(bind_mobile));
        assert!(!is_logged_in_url(bind_mobile));
        assert!(!is_editor_page(bind_mobile));

        let sso_bind = "https://sso.toutiao.com/bind_mobile_index/?redirect_url=https%3A%2F%2Fmp.toutiao.com%2Fprofile_v4%2Fgraphic%2Fpublish";
        assert!(is_login_url(sso_bind));
        assert!(!is_logged_in_url(sso_bind));
        assert!(!is_editor_page(sso_bind));
    }

    #[test]
    fn logged_in_url_detection() {
        assert!(is_logged_in_url(EDITOR_URL));
        // 登录页的 redirect_url 里也带着 `/profile_v4`，不能被误判成已登录。
        assert!(!is_logged_in_url(
            "https://mp.toutiao.com/auth/page/login/?redirect_url=https%3A%2F%2Fmp.toutiao.com%2Fprofile_v4%2Fgraphic%2Fpublish"
        ));
        assert!(!is_logged_in_url("https://mp.toutiao.com/"));
    }

    #[test]
    fn editor_page_detection_is_strict() {
        assert!(is_editor_page(EDITOR_URL));
        assert!(is_editor_page(
            "https://mp.toutiao.com/profile_v4/graphic/publish?pgc_id=7001"
        ));
        // 后台首页不是编辑页：页面级风控上下文不成立，注入必回 7050。
        assert!(!is_editor_page("https://mp.toutiao.com/profile_v4/index"));
        assert!(is_logged_in_url("https://mp.toutiao.com/profile_v4/index"));
        assert!(!is_editor_page("https://mp.toutiao.com/"));
    }

    #[test]
    fn status_prefix_parsing() {
        let (kind, payload) = parse_status("OPENPUB: DONE:{\"draftId\":\"7001\"}");
        assert_eq!(kind, "DONE");
        assert_eq!(payload, "{\"draftId\":\"7001\"}");

        let (k2, p2) = parse_status("OPENPUB: FAIL:参数校验失败 (err_no=7050)");
        assert_eq!(k2, "FAIL");
        assert_eq!(p2, "参数校验失败 (err_no=7050)");

        let (k3, p3) = parse_status("just a title");
        assert_eq!(k3, "UNKNOWN");
        assert_eq!(p3, "just a title");
    }

    /// 回归：真实事故现场。脚本已经写入终态、草稿也确实建好了
    /// （`draftId=7706817340284994048`），但头条编辑器周期性抢回
    /// `document.title`，Rust 侧 250ms 轮询始终没撞上那段可见空隙，
    /// 硬等 45 秒后报超时。改用 URL hash 通道后必须能解析出来。
    #[test]
    fn terminal_result_is_recovered_from_url_hash() {
        let url = "https://mp.toutiao.com/profile_v4/graphic/publish#openpub_res=DONE%3A%7B%22draftId%22%3A%227706817340284994048%22%2C%22publishedId%22%3Anull%7D";
        let body = status_from_url(url).expect("应能从 hash 取到结果");
        assert_eq!(
            body,
            "DONE:{\"draftId\":\"7706817340284994048\",\"publishedId\":null}"
        );
        let (draft_id, published_id) = terminal_result(&body).expect("应为终态").unwrap();
        assert_eq!(draft_id, "7706817340284994048");
        assert_eq!(published_id, None);
    }

    #[test]
    fn url_hash_carries_chinese_failure_message() {
        // 中文必须正确解码，否则日志里的失败原因会变成乱码。
        let url = "https://mp.toutiao.com/profile_v4/graphic/publish#openpub_res=FAIL%3A%E5%8F%82%E6%95%B0%E6%A0%A1%E9%AA%8C%E5%A4%B1%E8%B4%A5%20(err_no%3D7050)";
        let body = status_from_url(url).unwrap();
        assert_eq!(body, "FAIL:参数校验失败 (err_no=7050)");
        assert!(terminal_result(&body).unwrap().is_err());
    }

    #[test]
    fn url_without_result_hash_is_ignored() {
        // 普通编辑页 URL 不能被误判成已有结果。
        assert!(status_from_url(EDITOR_URL).is_none());
        assert!(status_from_url(
            "https://mp.toutiao.com/profile_v4/graphic/publish?pgc_id=123"
        )
        .is_none());
        // 空值也不应产出结果。
        assert!(status_from_url("https://mp.toutiao.com/x#openpub_res=").is_none());
    }

    #[test]
    fn running_state_is_not_terminal() {
        // 心跳不算终态，否则会在脚本刚注入时就误判完成。
        assert!(terminal_result("RUNNING").is_none());
        assert!(terminal_result("UNKNOWN").is_none());
    }

    #[test]
    fn published_result_keeps_publish_id() {
        let body = "DONE:{\"draftId\":\"7001\",\"publishedId\":\"7002\"}";
        let (draft_id, published_id) = terminal_result(body).unwrap().unwrap();
        assert_eq!(draft_id, "7001");
        assert_eq!(published_id.as_deref(), Some("7002"));
    }

    /// 回归：脚本每 50ms 把心跳写进标题（`OPENPUB: RUNNING`）。旧实现把无冒号
    /// 状态统一抹成 `UNKNOWN`，轮询第一圈就判定「未知状态」直接失败，于是真正
    /// 的失败原因全被吞掉，用户只看到一句 `UNKNOWN`。
    #[test]
    fn running_heartbeat_is_not_treated_as_failure() {
        let (kind, payload) = parse_status("OPENPUB: RUNNING");
        assert_eq!(kind, "RUNNING");
        assert!(payload.is_empty());
        assert_ne!(kind, "FAIL");
        assert_ne!(kind, "UNKNOWN");
    }

    #[test]
    fn draft_form_matches_plugin_field_set() {
        let request = sample_request(false);
        let form = article_form(&request, "<p>正文</p>", "0", false, "__AD_TYPE__");
        let names: Vec<&str> = form.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "article_type",
                "pgc_id",
                "source",
                "extra",
                "content",
                "title",
                "search_creation_info",
                "title_id",
                "mp_editor_stat",
                "is_refute_rumor",
                "save",
                "entrance",
                "timer_status",
                "timer_time",
                "educluecard",
                "draft_form_data",
                "pgc_feed_covers",
                "article_ad_type",
                "is_fans_article",
                "govern_forward",
                "praise",
                "disable_praise",
                "tree_plan_article",
                "star_order_id",
                "star_order_name",
                "activity_tag",
                "trends_writing_tag",
                "claim_exclusive",
            ]
        );
        assert_eq!(form.iter().find(|(k, _)| *k == "source").unwrap().1, "29");
        assert_eq!(form.iter().find(|(k, _)| *k == "save").unwrap().1, "0");
    }

    #[test]
    fn publish_form_adds_publish_only_fields() {
        let request = sample_request(true);
        let form = article_form(&request, "<p>正文</p>", "__PGC_ID__", true, "__AD_TYPE__");
        let names: Vec<&str> = form.iter().map(|(name, _)| *name).collect();
        assert!(names.contains(&"ic_uri_list"));
        assert!(names.contains(&"appid_list"));
        assert!(names.contains(&"stock_ids"));
        assert!(names.contains(&"concern_list"));
        assert_eq!(form.iter().find(|(k, _)| *k == "save").unwrap().1, "1");
        assert_eq!(form.iter().find(|(k, _)| *k == "entrance").unwrap().1, "main");
        // 发布步的 pgc_id 留占位符，由页面脚本回填刚拿到的草稿 ID。
        assert_eq!(form.iter().find(|(k, _)| *k == "pgc_id").unwrap().1, "__PGC_ID__");
    }

    #[test]
    fn fetch_script_contains_key_pieces() {
        let request = sample_request(false);
        let form = article_form(&request, "<p>正文</p>", "0", false, "__AD_TYPE__");
        let params = json!({
            "publishUrl": PUBLISH_URL,
            "adTypeUrl": AD_TYPE_URL,
            "draftForm": form,
            "publishForm": null,
        });
        let script = build_fetch_script(&params.to_string());
        assert!(script.contains("/mp/agw/article/publish"));
        assert!(script.contains("credentials: \"include\""));
        assert!(script.contains("application/x-www-form-urlencoded"));
        assert!(script.contains(STATUS_PREFIX));
        assert!(!script.contains("__OPENPUB_PARAMS__"));
    }
}