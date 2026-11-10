//! 知乎通道适配器（纯 DOM 填表 + 全自动发布）。
//!
//! 知乎的发布链路是「建草稿 → 进编辑器 → 填表 → 点发布」：
//! 建草稿必须在已登录的 `zhuanlan.zhihu.com` 页面上下文里发（同源 + Cookie），
//! 编辑器地址又依赖草稿 id，而「发布」只能由编辑器页面自己完成——
//! 发布接口 `https://www.zhihu.com/api/v4/content/publish` 需要 `x-zst-81` /
//! `x-zse-96` 运行时签名，且与 `zhuanlan.zhihu.com` 跨域，页面内 fetch 必被 CORS 拦下。
//!
//! 为什么不用「建草稿 + PATCH 写正文」：编辑器首屏加载的草稿快照会把 PATCH 写入
//! 的正文覆盖回旧内容，随后自动保存再把这个旧内容落盘，线上文章永远是旧的。
//! 因此正文与标题一律在编辑器页面里直接写进 DOM，交给知乎自己的自动保存落盘。
//!
//! 因此把注入脚本拆成两段：
//! - **Script A**（`build_draft_script`）：首页上下文里 `fetch` 建一个空草稿，
//!   成功后状态置 `DRAFT_READY:{id}`。
//! - **Script B**（`build_editor_script`）：编辑器页面里等标题框与正文框出现、
//!   DOM 填表、强制等 1500ms 让自动保存生效、校验编辑器标题与待发标题一致，
//!   然后点「发布」并处理二次确认，成功后状态置 `DONE`。
//!
//! 两段脚本都以 `window.__OPENPUB_STATUS__` 做幂等闸门，并持续把状态写进
//! `document.title`（`OPENPUB: {status}`）；Rust 侧轮询 `window.title()` 读取，
//! 彻底绕开 IPC、capability 与权限问题。
//!
//! 登录前置条件：知乎的扫码登录**复用本通道窗口**（`channel-zhihu`）。
//! 登录与发布必须落在同一个窗口实例上，因为 `open_channel_window` 才挂载了
//! `.on_new_window` 拦截——微信 / QQ 授权页走 `window.open`，没有这个拦截时
//! wry 会把 `NewWindowRequested` 标记为已处理却不做任何导航，表现为点击完全无反应。
//! 登录态由 WebView2 的用户数据目录记住，发布时同一个窗口直接复用这份 profile。
//!
//! 因此本通道**不承担「在同步流程里等用户扫码」的职责**：一旦轮询发现窗口被
//! 重定向到 `signin` / `login` 页面，立即失败并提示用户先去「打开平台页面」扫码。
//! 在同步流程里等扫码，只会让用户在页面跳转后丢掉已注入的脚本、体验割裂。
//!
//! 完整流程：
//! 1. 关掉上一轮遗留窗口，开窗到 `https://zhuanlan.zhihu.com/`。
//! 2. 跑 Script A，读到 `DRAFT_READY:{id}` 后把窗口导航到 `/p/{id}/edit`。
//! 3. 等编辑器地址就绪 + 2 秒渲染，注入 Script B。
//! 4. 读到 `DONE`（或页面自己跳到 `/p/{id}` 文章页）即视为发布成功，关窗返回。
//!
//! 更新场景（`existing_id` 有效）跳过 Script A，直接把窗口开到编辑页跑 Script B。

use std::time::Duration;

use tauri::{AppHandle, Manager, Url};
use tokio::time::{sleep, Instant};

use super::{channel_window_label, is_placeholder_post_id, open_channel_window};
use crate::platforms::shared::markdown;
use crate::platforms::{FailureClass, PublishOperation, PublishRequest, PublishResult};

pub const PLATFORM: &str = "zhihu";

/// 未登录时的统一提示：引导用户走独立登录通道，而不是在同步里被动扫码。
pub const NOT_LOGGED_IN_MESSAGE: &str = "知乎未登录，请先点击「打开平台页面」进行扫码登录";

/// 自动发布成功的提示。
pub const AUTO_PUBLISH_MESSAGE: &str = "已自动发布到知乎";

/// 扫码登录页。登录与发布共用同一个 `channel-zhihu` 窗口，只有这个窗口
/// 挂载了 `.on_new_window`，微信 / QQ 授权弹窗才能正常接管。
pub const LOGIN_URL: &str = "https://www.zhihu.com/signin";

/// 等待用户完成扫码的时长上限。
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// 建草稿的落脚页。纯接口路径不需要编辑器骨架，停在首页即可；
/// 同源（`zhuanlan.zhihu.com`）+ 带 Cookie，草稿接口不受 CORS 约束。
pub const BOOTSTRAP_URL: &str = "https://zhuanlan.zhihu.com/";

/// 编辑器地址前缀：`https://zhuanlan.zhihu.com/p/{id}/edit`。
pub const EDITOR_URL_PREFIX: &str = "https://zhuanlan.zhihu.com/p/";

const DRAFT_API_URL: &str = "https://zhuanlan.zhihu.com/api/articles/drafts";

/// 页面把状态写进 `document.title` 时统一加这个前缀，Rust 侧据此识别。
pub const STATUS_PREFIX: &str = "OPENPUB: ";

/// 填完内容后强制等待的时长：让知乎编辑器自己的自动保存把 DOM 内容落盘。
pub const EDITOR_SAVE_DELAY_MS: u64 = 1500;

/// 轮询窗口标题的间隔。脚本每 200ms 重刷一次标题，这里略快于脚本刷新，避免漏读。
const POLL_INTERVAL_MS: u64 = 200;

/// 等待 Script A 建草稿的总时长上限。
const DRAFT_TIMEOUT: Duration = Duration::from_secs(60);

/// 等待窗口导航到编辑器地址的时长上限。
const EDITOR_TIMEOUT: Duration = Duration::from_secs(60);

/// 地址到位后再等一会儿：编辑器骨架、正文与「发布」按钮还要挂载一轮。
const EDITOR_LOAD_DELAY: Duration = Duration::from_secs(2);

/// 等待 Script B 填表 + 点完发布的时长上限。
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(120);

/// 页面标题里没有状态前缀时的重新注入间隔，避免每轮轮询都 eval 一遍大脚本。
const REINJECT_INTERVAL: Duration = Duration::from_secs(1);

/// 关窗前留出的宽限时间，避免中断页面正在发出的发布请求。
const CLOSE_GRACE: Duration = Duration::from_millis(1000);

fn editor_url(draft_id: &str) -> String {
    format!("{}{}/edit", EDITOR_URL_PREFIX, draft_id)
}

fn is_login_url(url: &str) -> bool {
    url.contains("/signin") || url.contains("/login")
}

/// 判断地址是否已经「离开登录页、落到站内」。
///
/// 扫码（含微信 / QQ 授权）成功后知乎会把窗口从 `/signin` 送回首页
/// `https://www.zhihu.com/`，或直接落到 `zhuanlan.zhihu.com`。
/// 这是登录成功唯一的判据：不读 Cookie，因为通道窗口的 Cookie 由 WebView2
/// 自己持有，读它反而会引入与发布通道不一致的时序问题。
fn is_logged_in_url(url: &str) -> bool {
    if is_login_url(url) {
        return false;
    }
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    match parsed.host_str() {
        Some("zhuanlan.zhihu.com") => true,
        Some("www.zhihu.com") => {
            let path = parsed.path();
            path == "/" || path.is_empty()
        }
        _ => false,
    }
}

/// 判断地址是否已从编辑器 `/p/{id}/edit` 跳到文章页 `/p/{id}`。
///
/// 发布成功后知乎编辑器会自己跳转，文档被换掉、脚本与标题一起消失，
/// 因此这条判据是「DONE 状态读不到」时的兜底。
fn is_published_url(url: &str, draft_id: &str) -> bool {
    let Some(after) = url.split("/p/").nth(1) else {
        return false;
    };
    let path = after.split(['?', '#']).next().unwrap_or("");
    let mut segments = path.split('/');
    let id = segments.next().unwrap_or("");
    let tail = segments.next().unwrap_or("");
    id == draft_id && !tail.starts_with("edit")
}

/// 更新场景复用已有草稿 id，不再新建草稿；占位值一律拒收。
fn reusable_draft_id(request: &PublishRequest) -> Option<String> {
    let id = request.existing_id.as_deref()?.trim();
    if id.is_empty() || is_placeholder_post_id(id) {
        return None;
    }
    Some(id.to_string())
}

/// 打开 `channel-zhihu` 窗口到登录页，并在后台轮询导航结果。
///
/// 登录与发布共用同一个通道窗口：只有它挂载了 `.on_new_window`，
/// 微信 / QQ 授权弹窗才能正常弹出。登录成功后关闭窗口并发 `login-success`；
/// 窗口被用户提前关掉或超时则发 `login-failed`，让前端解除「窗口已打开」状态。
pub async fn open_login_channel(app: &AppHandle) -> Result<(), String> {
    let label = channel_window_label(PLATFORM);
    if app.get_webview_window(&label).is_some() {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
        // 关闭是异步的：必须等旧实例真正销毁，否则 `open_channel_window`
        // 会复用它，窗口还停在上一轮的页面上。
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.get_webview_window(&label).is_some() {
            if Instant::now() >= deadline {
                return Err("知乎通道窗口关闭超时，请稍后重试".to_string());
            }
            sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
    }

    open_channel_window(app, PLATFORM, LOGIN_URL)?;

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        watch_login(&app_handle).await;
    });
    Ok(())
}

/// 轮询通道窗口地址：从登录页跳到站内即视为登录成功。
async fn watch_login(app: &AppHandle) {
    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + LOGIN_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            crate::login::emit_failed(
                app,
                PLATFORM,
                "知乎登录超时，请重新点击「打开平台页面」扫码",
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
            if is_logged_in_url(url.as_str()) {
                crate::login::emit_success(app, PLATFORM);
                let _ = window.close();
                return;
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

pub async fn run(
    app: &AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> PublishResult {
    if let Err(result) = close_leftover_window(app, operation).await {
        return result;
    }

    let draft_id = match reusable_draft_id(request) {
        Some(id) => {
            // 更新场景：跳过 Script A，直接把窗口开到编辑页。
            let editor = editor_url(&id);
            if let Err(message) = open_channel_window(app, PLATFORM, &editor) {
                return PublishResult::failure(
                    FailureClass::NetworkError,
                    message,
                    operation,
                    false,
                );
            }
            id
        }
        None => match create_draft(app, request, operation).await {
            Ok(id) => id,
            Err(result) => return result,
        },
    };

    if let Err(result) = wait_editor_ready(app, &draft_id, operation).await {
        return result;
    }

    let content = markdown::to_html_content(&request.content);
    let script = match build_editor_script(&request.title, &content) {
        Ok(script) => script,
        Err(message) => {
            return PublishResult::failure(FailureClass::Unknown, message, operation, false)
        }
    };

    if let Err(result) = drive_editor(app, &draft_id, &script, operation).await {
        return result;
    }

    close_channel_window(app).await;

    PublishResult::success(
        draft_id.clone(),
        editor_url(&draft_id),
        AUTO_PUBLISH_MESSAGE,
        operation,
        true,
    )
}

/// 开窗 → 注入 Script A → 轮询标题拿到草稿 id → 导航到编辑页。
async fn create_draft(
    app: &AppHandle,
    request: &PublishRequest,
    operation: PublishOperation,
) -> Result<String, PublishResult> {
    let script = build_draft_script(&request.title).map_err(|message| {
        PublishResult::failure(FailureClass::Unknown, message, operation, false)
    })?;

    open_channel_window(app, PLATFORM, BOOTSTRAP_URL).map_err(|message| {
        PublishResult::failure(FailureClass::NetworkError, message, operation, false)
    })?;

    let draft_id = wait_draft_ready(app, &script, operation).await?;
    navigate_to_editor(app, &draft_id, operation)?;
    Ok(draft_id)
}

/// 轮询窗口标题，直到 Script A 报出 `DRAFT_READY:{id}` 或 `FAIL:`。
///
/// 被重定向到登录页时立即失败：登录是独立通道的职责，这里等下去只会让用户
/// 在同步流程里被动扫码，且扫码后的页面跳转会丢掉已注入的脚本。
async fn wait_draft_ready(
    app: &AppHandle,
    script: &str,
    operation: PublishOperation,
) -> Result<String, PublishResult> {
    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + DRAFT_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎通道等待草稿结果超时".to_string(),
                operation,
                false,
            ));
        }

        let Some(window) = app.get_webview_window(&label) else {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎通道窗口已被关闭，无法继续创建草稿".to_string(),
                operation,
                false,
            ));
        };

        if let Ok(url) = window.url() {
            if is_login_url(url.as_str()) {
                return Err(PublishResult::failure(
                    FailureClass::AuthInvalid,
                    NOT_LOGGED_IN_MESSAGE.to_string(),
                    operation,
                    false,
                ));
            }
            // 只在同源首页注入；登录跳转、跨站重定向时脚本无处落脚。
            if url.host_str() == Some("zhuanlan.zhihu.com") {
                // 重复注入是安全的：脚本以 `window.__OPENPUB_STATUS__` 做幂等闸门，
                // 已启动时直接 return。这样即使首帧文档还没稳定就注入、随后被导航
                // 换掉，下一轮也能在新文档里重新跑起来。
                if let Err(error) = window.eval(script) {
                    eprintln!("[zhihu] 注入建草稿脚本失败: {}", error);
                }
            }
        }

        if let Ok(title) = window.title() {
            if let Some(status) = title.strip_prefix(STATUS_PREFIX) {
                if let Some(id) = status.strip_prefix("DRAFT_READY:") {
                    let id = id.trim();
                    if !id.is_empty() && !is_placeholder_post_id(id) {
                        return Ok(id.to_string());
                    }
                } else if let Some(rest) = status.strip_prefix("FAIL:") {
                    return Err(PublishResult::failure(
                        FailureClass::PlatformError,
                        format!("知乎建草稿失败: {}", rest.trim()),
                        operation,
                        false,
                    ));
                }
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

fn navigate_to_editor(
    app: &AppHandle,
    draft_id: &str,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let Some(window) = app.get_webview_window(&channel_window_label(PLATFORM)) else {
        return Err(PublishResult::failure(
            FailureClass::NetworkError,
            "知乎通道窗口已被关闭，无法打开编辑页".to_string(),
            operation,
            false,
        ));
    };
    let url = Url::parse(&editor_url(draft_id)).map_err(|error| {
        PublishResult::failure(
            FailureClass::NetworkError,
            format!("知乎编辑页地址非法: {}", error),
            operation,
            false,
        )
    })?;
    window.navigate(url).map_err(|error| {
        PublishResult::failure(
            FailureClass::NetworkError,
            format!("打开知乎编辑页失败: {}", error),
            operation,
            false,
        )
    })
}

/// 等窗口真正落到 `/p/{id}/edit`，再留出编辑器渲染时间。
///
/// 这里只做「等地址到位」这一件事：不做强制 reload、不做复杂的 URL 变更重试。
/// 知乎编辑器是 SPA，地址一到位 DOM 骨架就已经在挂载，剩下的交给 Script B
/// 自己的 `waitForEditors` 轮询（它比 Rust 侧盲等更可靠，因为它能看到真实节点）。
async fn wait_editor_ready(
    app: &AppHandle,
    draft_id: &str,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let label = channel_window_label(PLATFORM);
    let marker = format!("/p/{}/edit", draft_id);
    let deadline = Instant::now() + EDITOR_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎编辑页加载超时".to_string(),
                operation,
                false,
            ));
        }

        let Some(window) = app.get_webview_window(&label) else {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎通道窗口已被关闭，无法进入编辑页".to_string(),
                operation,
                false,
            ));
        };

        if let Ok(url) = window.url() {
            let text = url.as_str();
            if is_login_url(text) {
                return Err(PublishResult::failure(
                    FailureClass::AuthInvalid,
                    NOT_LOGGED_IN_MESSAGE.to_string(),
                    operation,
                    false,
                ));
            }
            if text.contains(&marker) {
                break;
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }

    sleep(EDITOR_LOAD_DELAY).await;
    Ok(())
}

/// 在编辑页注入 Script B 并轮询，直到发布完成（`DONE` 或跳到 `/p/{id}` 文章页）。
///
/// 页面标题里没有 `OPENPUB:` 前缀说明脚本没跑起来或文档被换掉了，此时重新注入
/// （Script B 自带 `__OPENPUB_B_STARTED__` 闸门，重复注入不会重复填表）。
async fn drive_editor(
    app: &AppHandle,
    draft_id: &str,
    script: &str,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let label = channel_window_label(PLATFORM);
    let deadline = Instant::now() + PUBLISH_TIMEOUT;
    let mut last_inject: Option<Instant> = None;

    loop {
        if Instant::now() >= deadline {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎点击发布超时".to_string(),
                operation,
                false,
            ));
        }

        let Some(window) = app.get_webview_window(&label) else {
            return Err(PublishResult::failure(
                FailureClass::NetworkError,
                "知乎通道窗口已被关闭，无法继续操作编辑器".to_string(),
                operation,
                false,
            ));
        };

        if let Ok(url) = window.url() {
            let text = url.as_str();
            if is_login_url(text) {
                return Err(PublishResult::failure(
                    FailureClass::AuthInvalid,
                    NOT_LOGGED_IN_MESSAGE.to_string(),
                    operation,
                    false,
                ));
            }
            // 发布成功后编辑器会自己跳到 `/p/{id}`，文档被换掉、脚本与标题一起消失。
            // 把它当作成功的第二判据，避免页面跳得比 200ms 轮询还快时误报超时。
            if is_published_url(text, draft_id) {
                return Ok(());
            }
        }

        let title = window.title().unwrap_or_default();
        let has_status = title.starts_with(STATUS_PREFIX);

        // 标题里没有状态前缀：脚本没跑起来或页面被换掉了，重新注入一次。
        let need_inject =
            !has_status && last_inject.map_or(true, |at| at.elapsed() >= REINJECT_INTERVAL);
        if need_inject {
            if let Err(error) = window.eval(script) {
                eprintln!("[zhihu] 注入编辑器脚本失败: {}", error);
            }
            last_inject = Some(Instant::now());
        }

        if let Some(status) = title.strip_prefix(STATUS_PREFIX) {
            let status = status.trim();
            if status == "DONE" {
                return Ok(());
            }
            if let Some(rest) = status.strip_prefix("FAIL:") {
                return Err(PublishResult::failure(
                    FailureClass::PlatformError,
                    format!("知乎发布失败: {}", rest.trim()),
                    operation,
                    false,
                ));
            }
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

/// 关窗。留一点宽限时间，避免中断页面正在发出的发布请求。
async fn close_channel_window(app: &AppHandle) {
    sleep(CLOSE_GRACE).await;
    if let Some(window) = app.get_webview_window(&channel_window_label(PLATFORM)) {
        let _ = window.close();
    }
}

/// 关掉上一轮遗留的通道窗口。窗口会被复用同一个 label，不清干净的话
/// 这一轮的脚本会落在上一轮遗留的文档上（比如停在旧的编辑页）。
async fn close_leftover_window(
    app: &AppHandle,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let label = channel_window_label(PLATFORM);
    let Some(window) = app.get_webview_window(&label) else {
        return Ok(());
    };
    let _ = window.close();
    wait_window_destroyed(app, &label, operation).await
}

/// 关闭是异步的：`get_webview_window` 在事件循环处理完销毁前仍会返回 handle，
/// 必须等到返回 `None`，否则新窗口会复用旧实例（旧实例上还停着上一轮的编辑页）。
async fn wait_window_destroyed(
    app: &AppHandle,
    label: &str,
    operation: PublishOperation,
) -> Result<(), PublishResult> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if app.get_webview_window(label).is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(PublishResult::failure(
                FailureClass::Unknown,
                "知乎通道窗口关闭超时，无法进入发布阶段".to_string(),
                operation,
                false,
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// 生成 Script A。标题用 JSON 字面量内联，避免引号、换行破坏脚本。
pub fn build_draft_script(title: &str) -> Result<String, String> {
    let title_lit =
        serde_json::to_string(title).map_err(|error| format!("知乎标题序列化失败: {}", error))?;
    Ok(draft_script_body()
        .replace("__OPENPUB_TITLE__", &title_lit)
        .replace("__OPENPUB_DRAFT_API__", DRAFT_API_URL)
        .replace("__OPENPUB_STATUS_PREFIX__", STATUS_PREFIX))
}

/// Script A 模板：只建一个空草稿，正文交给 Script B 在编辑器里填。
///
/// 三条硬约束：
/// 1. **零 DOM 依赖业务**：不查节点、不设 value、不 execCommand，只发 `fetch`。
/// 2. **幂等**：同一文档内重复注入时，靠 `window.__OPENPUB_STATUS__` 直接 return。
/// 3. **自带地址闸门**：`/p/` 开头（编辑页）直接 return，避免在文章页重复建草稿。
///
/// 状态机：`START` → `DRAFT_READY:{id}`；任一环节失败写 `FAIL: {原因}`。
/// 状态同时写进 `window.__OPENPUB_STATUS__` 与 `document.title`（`OPENPUB: {status}`）。
pub fn draft_script_body() -> &'static str {
    r#"
(function () {
  try {
    var path = String(window.location.pathname || '');
    if (path.indexOf('/p/') === 0) { return; }
    if (window.__OPENPUB_STATUS__) { return; }

    window.__OPENPUB_STATUS__ = 'START';

    // 页面框架会反复改写 document.title，这里持续重刷，保证 Rust 侧轮询能读到最新状态。
    setInterval(function () {
      try { document.title = '__OPENPUB_STATUS_PREFIX__' + window.__OPENPUB_STATUS__; } catch (e) {}
    }, 200);
    try { document.title = '__OPENPUB_STATUS_PREFIX__' + window.__OPENPUB_STATUS__; } catch (e) {}

    var title = __OPENPUB_TITLE__;

    function setStatus(status) {
      window.__OPENPUB_STATUS__ = status;
      try { document.title = '__OPENPUB_STATUS_PREFIX__' + status; } catch (e) {}
    }

    fetch('__OPENPUB_DRAFT_API__', {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json', 'x-requested-with': 'fetch' },
      body: JSON.stringify({ title: title, content: '', delta_time: 0 })
    }).then(function (res) {
      if (!res.ok) { throw new Error('建草稿 HTTP ' + res.status); }
      return res.json();
    }).then(function (json) {
      var id = json && json.id !== undefined && json.id !== null ? String(json.id) : '';
      if (!id) { throw new Error('建草稿响应缺少 id'); }
      setStatus('DRAFT_READY:' + id);
    }).catch(function (err) {
      setStatus('FAIL: ' + (err && err.message ? err.message : String(err)));
    });
  } catch (e) {
    try {
      window.__OPENPUB_STATUS__ = 'FAIL: ' + (e && e.message ? e.message : String(e));
      document.title = '__OPENPUB_STATUS_PREFIX__' + window.__OPENPUB_STATUS__;
    } catch (e2) {}
  }
})();
"#
}

/// 生成 Script B。标题 / 正文 / 自动保存等待时长都内联为字面量。
pub fn build_editor_script(title: &str, content: &str) -> Result<String, String> {
    let title_lit =
        serde_json::to_string(title).map_err(|error| format!("知乎标题序列化失败: {}", error))?;
    let content_lit =
        serde_json::to_string(content).map_err(|error| format!("知乎正文序列化失败: {}", error))?;
    Ok(editor_script_body()
        .replace("__OPENPUB_TITLE__", &title_lit)
        .replace("__OPENPUB_CONTENT__", &content_lit)
        .replace("__OPENPUB_SAVE_DELAY__", &EDITOR_SAVE_DELAY_MS.to_string())
        .replace("__OPENPUB_STATUS_PREFIX__", STATUS_PREFIX))
}

/// Script B 模板：在编辑页里 DOM 填表，然后自动点「发布」。
///
/// 关键约束：
/// 1. **填表先于发布**：等标题框与正文框出现，用 `execCommand('insertText')` /
///    `insertHTML` / `innerHTML` 分层写入，写完校验正文是否真的进了 DOM。
/// 2. **强制等自动保存**：填完后固定等 1500ms，让知乎自己的自动保存把 DOM 内容落盘；
///    直接点发布会发出去一份尚未保存的快照。
/// 3. **发布前校验标题**：编辑器标题必须等于待发标题，否则写 `FAIL` 且绝不点发布。
/// 4. **一次点击 + 二次确认**：主按钮文案「发布」，弹窗再点「确定 / 确认」。
///
/// 状态机：`FILLING` → `PUBLISHING` → `CLICKED` → `DONE`；失败写 `FAIL: {原因}`。
/// 以 `window.__OPENPUB_B_STARTED__` 做幂等闸门，Rust 侧重复注入不会重复填表或重复点击。
pub fn editor_script_body() -> &'static str {
    r#"
(function () {
  try {
    if (window.__OPENPUB_B_STARTED__) { return; }
    window.__OPENPUB_B_STARTED__ = true;

    var TITLE = __OPENPUB_TITLE__;
    var CONTENT_HTML = __OPENPUB_CONTENT__;
    var SAVE_DELAY = __OPENPUB_SAVE_DELAY__;
    var FIND_TIMEOUT = 30000;
    var CONFIRM_TIMEOUT = 8000;
    var startedAt = Date.now();

    function setStatus(status) {
      window.__OPENPUB_STATUS__ = status;
      try { document.title = '__OPENPUB_STATUS_PREFIX__' + status; } catch (e) {}
    }

    setInterval(function () {
      try { document.title = '__OPENPUB_STATUS_PREFIX__' + window.__OPENPUB_STATUS__; } catch (e) {}
    }, 200);
    setStatus('FILLING');

    function textOf(el) {
      return String((el && el.textContent) || '').replace(/\s+/g, '');
    }

    function visible(el) {
      if (!el || el.disabled) { return false; }
      var style = window.getComputedStyle(el);
      if (style && (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0')) { return false; }
      return el.offsetParent !== null || (style && style.position === 'fixed');
    }

    function firstVisible(selectors, skipTitleLike) {
      for (var i = 0; i < selectors.length; i++) {
        var nodes = document.querySelectorAll(selectors[i]);
        for (var j = 0; j < nodes.length; j++) {
          var el = nodes[j];
          if (!visible(el)) { continue; }
          if (skipTitleLike) {
            var cls = String(el.className || '').toLowerCase();
            if (cls.indexOf('title') >= 0) { continue; }
            if (el.closest && el.closest('input, textarea')) { continue; }
          }
          return el;
        }
      }
      return null;
    }

    function findTitleInput() {
      return firstVisible([
        'input.WriteIndex-titleInput',
        'input[placeholder*="标题"]',
        'textarea[placeholder*="标题"]',
        '.WriteIndex-titleInput input',
        '.WriteIndex-titleInput textarea'
      ], false);
    }

    function findBodyEditor() {
      return firstVisible([
        '.ProseMirror[contenteditable="true"]',
        '.DraftEditor-root [contenteditable="true"]',
        '[contenteditable="true"][role="textbox"]',
        '.WriteIndex-content [contenteditable="true"]',
        '[contenteditable="true"]'
      ], true);
    }

    function readTitle(el) {
      if (!el) { return ''; }
      if (el.value !== undefined && el.value !== null) { return String(el.value); }
      return String(el.textContent || '');
    }

    function selectAllIn(el) {
      try {
        el.focus();
        if (el.select) { el.select(); return; }
        var range = document.createRange();
        range.selectNodeContents(el);
        var selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
      } catch (e) {}
    }

    function setNativeValue(el, value) {
      var proto = Object.getPrototypeOf(el);
      var desc = proto ? Object.getOwnPropertyDescriptor(proto, 'value') : null;
      if (desc && desc.set) { desc.set.call(el, value); }
      else { el.value = value; }
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    }

    function fillTitle(el) {
      if (readTitle(el) === TITLE) { return true; }
      selectAllIn(el);
      var inserted = false;
      try { inserted = document.execCommand('insertText', false, TITLE); } catch (e) { inserted = false; }
      if (!inserted || readTitle(el) !== TITLE) { setNativeValue(el, TITLE); }
      return readTitle(el) === TITLE;
    }

    // 正文的纯文本探针：HTML 写进 DOM 后用它校验内容确实进了编辑器。
    var probe = (function () {
      var holder = document.createElement('div');
      holder.innerHTML = CONTENT_HTML;
      return String(holder.textContent || '').replace(/\s+/g, '');
    })();

    function bodyFilled(el) {
      var actual = String(el.textContent || '').replace(/\s+/g, '');
      return probe ? actual.indexOf(probe) >= 0 : actual.length > 0;
    }

    function fillBody(el) {
      if (bodyFilled(el)) { return true; }
      var attempts = [
        function () {
          selectAllIn(el);
          try { return document.execCommand('insertHTML', false, CONTENT_HTML); } catch (e) { return false; }
        },
        function () {
          selectAllIn(el);
          el.innerHTML = CONTENT_HTML;
          el.dispatchEvent(new Event('input', { bubbles: true }));
          return true;
        },
        function () {
          selectAllIn(el);
          try { return document.execCommand('insertText', false, probe); } catch (e) { return false; }
        }
      ];
      for (var i = 0; i < attempts.length; i++) {
        try { attempts[i](); } catch (e) {}
        if (bodyFilled(el)) { return true; }
      }
      return bodyFilled(el);
    }

    function findPublishButton() {
      var nodes = document.querySelectorAll('button, [role="button"]');
      var loose = null;
      for (var i = 0; i < nodes.length; i++) {
        var el = nodes[i];
        if (!visible(el)) { continue; }
        var text = textOf(el);
        if (text === '发布') { return el; }
        if (!loose && text.indexOf('发布') >= 0) { loose = el; }
      }
      if (loose) { return loose; }
      // 兜底：知乎编辑器的主按钮固定带 Button--primary。
      var primaries = document.querySelectorAll('button.Button--primary');
      for (var j = 0; j < primaries.length; j++) {
        if (visible(primaries[j])) { return primaries[j]; }
      }
      return null;
    }

    function findConfirmButton(clicked) {
      var scope = document.querySelector('[role="dialog"], .Modal, .Modal-content, .Modal-inner');
      var inDialog = !!scope;
      if (!scope) { scope = document; }
      var nodes = scope.querySelectorAll('button, [role="button"]');
      // 只有在弹窗内部才接受裸「发布」文案；整页兜底时只认「确定 / 确认」，
      // 否则会把刚点过的主按钮再点一次。
      var order = inDialog
        ? ['确定发布', '确认发布', '发布', '确定', '确认']
        : ['确定发布', '确认发布', '确定', '确认'];
      for (var k = 0; k < order.length; k++) {
        for (var i = 0; i < nodes.length; i++) {
          var el = nodes[i];
          if (el === clicked) { continue; }
          if (!visible(el)) { continue; }
          if (textOf(el) === order[k]) { return el; }
        }
      }
      return null;
    }

    var mainButton = null;
    var confirmAt = 0;
    var publishStartedAt = 0;

    // 点完主按钮后等二次确认弹窗。弹窗慢一点也要等，等不到再认为「一次点击已直接发布」。
    function awaitConfirm() {
      try {
        var confirm = findConfirmButton(mainButton);
        if (confirm) {
          confirm.click();
          setStatus('DONE');
          return;
        }
        if (Date.now() - confirmAt > CONFIRM_TIMEOUT) {
          setStatus('DONE');
          return;
        }
        setTimeout(awaitConfirm, 500);
      } catch (e) {
        setStatus('FAIL: ' + (e && e.message ? e.message : String(e)));
      }
    }

    function findAndClickPublish() {
      try {
        var button = findPublishButton();
        if (!button) {
          if (!publishStartedAt) { publishStartedAt = Date.now(); }
          if (Date.now() - publishStartedAt > FIND_TIMEOUT) {
            setStatus('FAIL: 未找到知乎「发布」按钮');
            return;
          }
          setTimeout(findAndClickPublish, 500);
          return;
        }
        mainButton = button;
        button.click();
        setStatus('CLICKED');
        confirmAt = Date.now();
        setTimeout(awaitConfirm, 1500);
      } catch (e) {
        setStatus('FAIL: ' + (e && e.message ? e.message : String(e)));
      }
    }

    // 填完后统一收口：先校验编辑器标题，再点发布。
    function afterFill() {
      try {
        var titleEl = findTitleInput();
        var actual = readTitle(titleEl);
        if (actual !== TITLE) {
          setStatus('FAIL: 编辑器标题与待发标题不一致');
          return;
        }
        setStatus('PUBLISHING');
        findAndClickPublish();
      } catch (e) {
        setStatus('FAIL: ' + (e && e.message ? e.message : String(e)));
      }
    }

    function waitForEditors() {
      try {
        var titleEl = findTitleInput();
        var bodyEl = findBodyEditor();
        if (!titleEl || !bodyEl) {
          if (Date.now() - startedAt > FIND_TIMEOUT) {
            setStatus('FAIL: 未找到知乎编辑器的标题或正文输入框');
            return;
          }
          setTimeout(waitForEditors, 500);
          return;
        }
        if (!fillTitle(titleEl) || !fillBody(bodyEl)) {
          setStatus('FAIL: 知乎编辑器内容填充失败');
          return;
        }
        // 填完强制等 1500ms，让知乎的自动保存把 DOM 内容落盘后再校验 / 发布。
        setTimeout(afterFill, SAVE_DELAY);
      } catch (e) {
        setStatus('FAIL: ' + (e && e.message ? e.message : String(e)));
      }
    }

    waitForEditors();
  } catch (e) {
    try {
      window.__OPENPUB_STATUS__ = 'FAIL: ' + (e && e.message ? e.message : String(e));
      document.title = '__OPENPUB_STATUS_PREFIX__' + window.__OPENPUB_STATUS__;
    } catch (e2) {}
  }
})();
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_draft_script() -> String {
        build_draft_script("标题").expect("建草稿脚本应能生成")
    }

    fn sample_editor_script() -> String {
        build_editor_script("标题", "<p>正文</p>").expect("编辑器脚本应能生成")
    }

    #[test]
    fn draft_script_only_creates_an_empty_draft() {
        let script = sample_draft_script();

        // 建草稿走同源 fetch，正文留空交给 Script B 在编辑器里填。
        assert!(script.contains(DRAFT_API_URL));
        assert!(script.contains("method: 'POST'"));
        assert!(script.contains("content: ''"));
        assert!(script.contains("delta_time: 0"));
        assert!(script.contains("credentials: 'include'"));

        // 不再 PATCH 写正文：那会被编辑器首屏快照覆盖。
        assert!(!script.contains("PATCH"));
        assert!(!script.contains("'/draft'"));

        // Script A 零 DOM 业务依赖：不查节点、不写 value、不模拟点击。
        assert!(!script.contains("querySelector"));
        assert!(!script.contains("getElementsBy"));
        assert!(!script.contains("contenteditable"));
        assert!(!script.contains("execCommand"));
        assert!(!script.contains("setNativeValue"));
        assert!(!script.contains("DraftEditor"));
        assert!(!script.contains("Button--primary"));
        assert!(!script.contains("getComputedStyle"));
        assert!(!script.contains("document.body"));
    }

    #[test]
    fn draft_script_reports_draft_ready_after_create() {
        let script = sample_draft_script();

        assert!(script.contains("window.__OPENPUB_STATUS__ = 'START'"));
        assert!(script.contains("setStatus('DRAFT_READY:' + id)"));
        assert!(script.contains("setStatus('FAIL: '"));
        // Script A 不再自己宣布 DONE：发布动作已经拆到 Script B。
        assert!(!script.contains("setStatus('DONE')"));
        assert!(script.contains("document.title = 'OPENPUB: ' + status"));
    }

    #[test]
    fn draft_script_is_idempotent_and_guards_editor_page() {
        let script = sample_draft_script();
        assert!(script.contains("if (window.__OPENPUB_STATUS__) { return; }"));
        assert!(script.contains("if (path.indexOf('/p/') === 0) { return; }"));
    }

    #[test]
    fn draft_script_inlines_title_as_json_literal() {
        let script = sample_draft_script();
        assert!(script.contains("\"标题\""));
        assert!(!script.contains("__OPENPUB_TITLE__"));
        assert!(!script.contains("__OPENPUB_DRAFT_API__"));
        assert!(!script.contains("__OPENPUB_STATUS_PREFIX__"));
    }

    #[test]
    fn draft_script_escapes_quotes_and_newlines() {
        let script = build_draft_script("含\"引号\"\n第二行").expect("应能生成");
        assert!(script.contains("含\\\"引号\\\""));
        assert!(script.contains("\\n第二行"));
    }

    #[test]
    fn editor_script_fills_title_and_body_through_dom() {
        let script = sample_editor_script();

        // 等标题框与正文框出现。
        assert!(script.contains("function findTitleInput()"));
        assert!(script.contains("input[placeholder*=\"标题\"]"));
        assert!(script.contains("function findBodyEditor()"));
        assert!(script.contains("contenteditable=\"true\""));

        // 填标题：execCommand insertText 为主，原生 setter 兜底。
        assert!(script.contains("document.execCommand('insertText', false, TITLE)"));
        assert!(script.contains("function setNativeValue(el, value)"));
        assert!(script.contains("new Event('input', { bubbles: true })"));

        // 填正文：insertHTML 为主，innerHTML / insertText 兜底。
        assert!(script.contains("document.execCommand('insertHTML', false, CONTENT_HTML)"));
        assert!(script.contains("el.innerHTML = CONTENT_HTML"));
        assert!(script.contains("document.execCommand('insertText', false, probe)"));
        assert!(script.contains("function bodyFilled(el)"));
    }

    #[test]
    fn editor_script_waits_for_autosave_before_verifying() {
        let script = sample_editor_script();
        assert_eq!(EDITOR_SAVE_DELAY_MS, 1500);
        assert!(script.contains("var SAVE_DELAY = 1500;"));
        assert!(script.contains("setTimeout(afterFill, SAVE_DELAY)"));
        assert!(!script.contains("__OPENPUB_SAVE_DELAY__"));
    }

    #[test]
    fn editor_script_validates_title_before_publish() {
        let script = sample_editor_script();
        assert!(script.contains("function afterFill()"));
        assert!(script.contains("if (actual !== TITLE)"));
        assert!(script.contains("setStatus('FAIL: 编辑器标题与待发标题不一致')"));
        // 校验通过后才可能进入发布分支。
        assert!(script.contains("setStatus('PUBLISHING')"));
        assert!(script.contains("findAndClickPublish()"));
    }

    #[test]
    fn editor_script_always_publishes_without_a_flag() {
        let script = sample_editor_script();
        // 回归「极速全自动」：不再有「只填表不发布」的分支。
        assert!(!script.contains("SHOULD_PUBLISH"));
        assert!(!script.contains("__OPENPUB_PUBLISH__"));
        assert!(!script.contains("setStatus('FILLED')"));
        assert!(script.contains("findAndClickPublish();"));
    }

    #[test]
    fn editor_script_inlines_payload_as_json_literals() {
        let script = sample_editor_script();
        assert!(script.contains("\"标题\""));
        assert!(script.contains("\"<p>正文</p>\""));
        assert!(!script.contains("__OPENPUB_TITLE__"));
        assert!(!script.contains("__OPENPUB_CONTENT__"));
        assert!(!script.contains("__OPENPUB_STATUS_PREFIX__"));
    }

    #[test]
    fn editor_script_escapes_quotes_and_newlines() {
        let script = build_editor_script("含\"引号\"", "<p>第一行\n第二行</p>").expect("应能生成");
        assert!(script.contains("含\\\"引号\\\""));
        assert!(script.contains("第一行\\n第二行"));
    }

    #[test]
    fn editor_script_clicks_primary_button_then_confirms() {
        let script = sample_editor_script();

        // 找按钮：先按文本「发布」精确匹配，再放宽到包含「发布」，最后兜底 Button--primary。
        assert!(script.contains("querySelectorAll('button, [role=\"button\"]')"));
        assert!(script.contains("text === '发布'"));
        assert!(script.contains("button.Button--primary"));
        assert!(script.contains("function findPublishButton()"));

        // 二次确认弹窗。
        assert!(script.contains("function findConfirmButton(clicked)"));
        assert!(script.contains("if (el === clicked) { continue; }"));
        assert!(script.contains("'确定发布'"));
        assert!(script.contains("'确认发布'"));
        assert!(script.contains("setTimeout(awaitConfirm, 1500)"));

        // 点完主按钮后置 CLICKED，二次确认成功后置 DONE。
        assert!(script.contains("setStatus('CLICKED')"));
        assert!(script.contains("setStatus('DONE')"));
        assert!(script.contains("OPENPUB: "));
    }

    #[test]
    fn editor_script_is_idempotent() {
        let script = sample_editor_script();
        assert!(script.contains("if (window.__OPENPUB_B_STARTED__) { return; }"));
        assert!(script.contains("window.__OPENPUB_B_STARTED__ = true"));
    }

    #[test]
    fn scripts_never_call_publish_api_or_ipc() {
        let draft = sample_draft_script();
        let editor = sample_editor_script();
        for script in [&draft, &editor] {
            // 发布接口需要运行时签名，脚本里绝不能出现。
            assert!(!script.contains("api/v4/content/publish"));
            // 不依赖 Tauri IPC / 自定义命令。
            assert!(!script.contains("__TAURI__"));
            assert!(!script.contains("invoke"));
            assert!(!script.contains("postMessage"));
        }
    }

    #[test]
    fn editor_url_matches_publish_flow() {
        assert_eq!(
            editor_url("123456"),
            "https://zhuanlan.zhihu.com/p/123456/edit"
        );
    }

    #[test]
    fn is_published_url_distinguishes_editor_from_article() {
        // 还停在编辑页：不算发布完成。
        assert!(!is_published_url(
            "https://zhuanlan.zhihu.com/p/123/edit",
            "123"
        ));
        // 编辑器跳到文章页：发布成功。
        assert!(is_published_url("https://zhuanlan.zhihu.com/p/123", "123"));
        assert!(is_published_url(
            "https://zhuanlan.zhihu.com/p/123?utm=1",
            "123"
        ));
        // 别的文章、首页、非 /p/ 地址都不算。
        assert!(!is_published_url("https://zhuanlan.zhihu.com/p/999", "123"));
        assert!(!is_published_url("https://zhuanlan.zhihu.com/", "123"));
        assert!(!is_published_url("https://zhuanlan.zhihu.com/p/12345", "123"));
    }

    #[test]
    fn draft_script_is_never_injected_on_editor_page() {
        // Script A 自带 `/p/` 地址闸门，更新场景直接开编辑页时不会重复建草稿。
        let script = sample_draft_script();
        assert!(script.contains("path.indexOf('/p/') === 0"));
    }

    #[test]
    fn not_logged_in_message_points_to_login_channel() {
        assert!(NOT_LOGGED_IN_MESSAGE.contains("未登录"));
        assert!(NOT_LOGGED_IN_MESSAGE.contains("打开平台页面"));
        assert!(NOT_LOGGED_IN_MESSAGE.contains("扫码"));
    }

    #[test]
    fn login_urls_are_detected() {
        assert!(is_login_url("https://www.zhihu.com/signin"));
        assert!(is_login_url("https://www.zhihu.com/signin?next=%2F&utm=x"));
        assert!(is_login_url("https://zhuanlan.zhihu.com/login"));
        assert!(!is_login_url("https://zhuanlan.zhihu.com/p/123/edit"));
        assert!(!is_login_url("https://zhuanlan.zhihu.com/"));
    }

    #[test]
    fn logged_in_url_means_left_the_login_page() {
        // 扫码成功后知乎会把窗口送回首页或 zhuanlan。
        assert!(is_logged_in_url("https://www.zhihu.com/"));
        assert!(is_logged_in_url("https://www.zhihu.com"));
        assert!(is_logged_in_url("https://www.zhihu.com/?utm_source=x"));
        assert!(is_logged_in_url("https://zhuanlan.zhihu.com/"));
        assert!(is_logged_in_url("https://zhuanlan.zhihu.com/write"));
        // 停在登录页、非知乎域名、站内二级页都不算登录成功。
        assert!(!is_logged_in_url("https://www.zhihu.com/signin"));
        assert!(!is_logged_in_url("https://www.zhihu.com/signin?next=%2F"));
        assert!(!is_logged_in_url("https://zhuanlan.zhihu.com/login"));
        assert!(!is_logged_in_url("https://open.weixin.qq.com/connect/qrconnect"));
        assert!(!is_logged_in_url("https://www.zhihu.com/question/123"));
    }

    #[test]
    fn login_channel_uses_the_same_window_label_as_publish() {
        // 登录与发布必须共用同一个窗口实例，否则 `.on_new_window` 拦截不在场。
        assert_eq!(channel_window_label(PLATFORM), "channel-zhihu");
        assert!(LOGIN_URL.starts_with("https://www.zhihu.com/signin"));
        assert!(is_login_url(LOGIN_URL));
        assert!(!is_logged_in_url(LOGIN_URL));
    }
}