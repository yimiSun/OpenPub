//! Webview 通道共享层。
//!
//! 通道窗口 label 固定为 `channel-{platform}`，与登录窗口 `login-{platform}` 分离。
//! 见 `design.md` Decision 3。
//!
//! 这里只保留三类通用能力：
//! 1. 开窗 / 复用窗口（`open_channel_window`），并把文档标题同步到原生窗口标题；
//! 2. 让窗口自己导航到目标地址的脚本（`navigate_script`，用于接管 `window.open`）；
//! 3. 把「新窗口请求改为当前窗口内导航」的拦截抽成公共能力
//!    （`with_same_window_navigation`），通道窗口与登录窗口共用同一份实现。
//!
//! 各平台的「注入什么脚本、怎么读结果」一律留在自己的适配器里（例如知乎用
//! `window.eval` 注入纯 JS、以 `document.title` 轮询状态）。共享层刻意不再提供
//! IPC 回传与 `initialization_script` 注入：远程页面里的 `__TAURI_INTERNALS__`
//! 不可靠，那条路径已实测废弃，留着只会把后续平台引向必失败的实现。

use std::path::PathBuf;

use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

pub mod douyin;
pub mod jianshu;
pub mod segmentfault;
pub mod toutiao;
pub mod xiaohongshu;
pub mod zhihu;

/// 通道窗口 label 固定为 `channel-{platform}`，与登录窗口 `login-{platform}` 分离。
pub const CHANNEL_WINDOW_PREFIX: &str = "channel-";

const PLACEHOLDER_VALUES: &[&str] = &[
    "", "0", "-", "placeholder", "unknown", "todo", "tbd", "null", "none", "n/a", "na",
];

pub fn is_placeholder_platform(platform: &str) -> bool {
    PLACEHOLDER_VALUES.contains(&platform.trim().to_ascii_lowercase().as_str())
}

pub fn is_placeholder_post_id(post_id: &str) -> bool {
    PLACEHOLDER_VALUES.contains(&post_id.trim().to_ascii_lowercase().as_str())
}

pub fn channel_window_label(platform: &str) -> String {
    format!("{}{}", CHANNEL_WINDOW_PREFIX, platform.trim())
}

/// 显式绑定 WebView2 的用户数据目录，登录窗口与通道窗口共用。
///
/// Tauri 2 在 Windows 上确实会给未指定的 webview 默认填 `app_local_data_dir()`，
/// 但那是「解析成功才填」：解析失败时它只打一行 log，然后退回 WebView2 自己的
/// 目录（不在本应用目录下），表现就是「重启后所有平台掉登录」。因此这里不依赖
/// 隐式默认值，显式传入同一个路径。
///
/// **路径不得改动**（尤其不要加 `webview_data` 之类的子目录）：
/// `data_directory` 直接决定 WebView2 的 profile 位置，换目录等于换 profile，
/// 已登录的知乎 / 简书 / 小红书 / 头条会当场全部失效，必须重新扫码。
/// 当前值解析为 `%LOCALAPPDATA%\com.openpub.desktop`，WebView2 会在其下建
/// `EBWebView`，与 Tauri 的默认行为逐字节一致，因此显式化不会迁移任何数据。
///
/// 另一个必须共用的理由：登录窗口抓到的 Cookie 要能被通道窗口看到。两者若用
/// 不同目录就是两个 profile，登录窗口的授权结果在通道窗口里根本不存在。
pub fn webview_data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("解析应用本地数据目录失败: {}", error))?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("创建 Webview 数据目录失败: {}", error))?;
    }
    Ok(dir)
}

/// 生成「当前窗口跳到目标地址」的注入脚本。
///
/// 地址一律走 `serde_json::to_string` 转成 JSON 字符串字面量再拼进 JS，
/// 避免地址里的引号、反斜杠或换行破坏脚本结构。
pub fn navigate_script(target: &str) -> Result<String, String> {
    let literal = serde_json::to_string(target)
        .map_err(|error| format!("跳转地址序列化失败: {}", error))?;
    Ok(format!("window.location.href = {};", literal))
}

/// 「`target="_blank"` 链接改为当前窗口打开」的初始化脚本。
///
/// 简书等平台的「微信 / QQ 登录」不是 `window.open`，而是带 `target="_blank"` 的
/// 普通 `<a>`（实测形态 `<a id="weixin" target="_blank" href="/users/auth/wechat">`）。
/// 这类链接点击后由 WebView2 直接抛 NewWindowRequested，`.on_new_window` 只能
/// 拒绝新窗口、再由 Rust 侧补一次 `window.location.href` 跳转；请求已经先被
/// 浏览器接管，实测表现为「点了完全没反应」。
///
/// 这里在文档开始解析前就接管点击：命中 `target="_blank"` 的 `<a>` 时先
/// `preventDefault()`，再用 `window.location.href` 在当前窗口内打开，请求根本
/// 不会走到 NewWindowRequested。脚本是纯 DOM 操作，不依赖 `__TAURI_INTERNALS__`
/// （那条 IPC 回传路径已实测废弃）。
///
/// 双保险：除了点击拦截，还会把页面上已存在的 `target="_blank"` 就地改写为
/// `_self`，覆盖脚本直接 `.click()`、表单提交等绕过 click 监听的路径。
pub const SAME_WINDOW_LINK_SCRIPT: &str = r#"(function () {
  if (window.__OPENPUB_SAME_WINDOW_LINKS__) { return; }
  window.__OPENPUB_SAME_WINDOW_LINKS__ = true;

  function isHttpUrl(value) {
    var lowered = String(value || '').toLowerCase();
    return lowered.indexOf('http://') === 0 || lowered.indexOf('https://') === 0;
  }

  // 头条 / 简书等登录页底部的「微信 / QQ 登录」普遍走 `window.open`，
  // 而不是 `target="_blank"` 的普通链接。未劫持时这类调用会落到
  // NewWindowRequested：wry 把弹窗标记为已处理却不导航，表现就是
  // 「点了完全没反应」。这里在页面代码执行前就把 `window.open` 换成
  // 「在当前窗口内导航」，请求根本不会走到弹窗路径。
  //
  // 返回 `window` 而不是 `null`：部分 OAuth 代码会立刻访问返回值的
  // `.closed` / `.focus()`，返回 `null` 会抛 TypeError；返回自身则只是
  // 无害的空操作（此时当前文档已经导航走了）。
  var nativeOpen = window.open;
  var hijackedOpen = function (url, target, features) {
    if (!url) {
      return typeof nativeOpen === 'function'
        ? nativeOpen.call(window, url, target, features)
        : null;
    }
    var resolved = '';
    try { resolved = new URL(String(url), location.href).href; } catch (error) { resolved = ''; }
    if (resolved && isHttpUrl(resolved)) {
      try { location.href = resolved; } catch (error) {}
      return window;
    }
    // 非 http(s)（`about:blank`、`javascript:`）交给原生实现，由 Rust 侧的
    // `on_new_window` 拒绝，绝不让页面通过 URL 直接执行脚本。
    return typeof nativeOpen === 'function'
      ? nativeOpen.call(window, url, target, features)
      : null;
  };
  function ensureOpenHijacked() {
    if (window.open !== hijackedOpen) { window.open = hijackedOpen; }
  }
  ensureOpenHijacked();

  function isBlankTarget(anchor) {
    var target = (anchor.getAttribute('target') || '').trim().toLowerCase();
    return target === '_blank' || target === '_new';
  }

  function anchorOf(node) {
    while (node && node.nodeType !== 1) { node = node.parentNode; }
    while (node) {
      if (node.tagName && node.tagName.toUpperCase() === 'A') { return node; }
      node = node.parentElement;
    }
    return null;
  }

  document.addEventListener('click', function (event) {
    var anchor = anchorOf(event.target);
    if (!anchor || !isBlankTarget(anchor)) { return; }
    var href = anchor.getAttribute('href') || '';
    if (!href || href.charAt(0) === '#') { return; }
    if (href.toLowerCase().indexOf('javascript:') === 0) { return; }
    event.preventDefault();
    event.stopPropagation();
    window.location.href = anchor.href;
  }, true);

  function normalizeTargets() {
    try {
      var links = document.querySelectorAll('a[target]');
      for (var i = 0; i < links.length; i++) {
        if (isBlankTarget(links[i])) { links[i].setAttribute('target', '_self'); }
      }
    } catch (error) {}
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', normalizeTargets);
  } else {
    normalizeTargets();
  }
  window.addEventListener('load', normalizeTargets);
  // 页面可能在运行中再次覆盖 `window.open`（部分登录 SDK 会这样做），
  // 每轮复查一次，保证劫持始终有效。
  setInterval(function () {
    normalizeTargets();
    ensureOpenHijacked();
  }, 2000);
})();"#;

/// 给任意窗口挂上「新窗口请求改为当前窗口内导航」的拦截能力。
///
/// 登录页与发布页上的「微信 / QQ 登录」普遍走 `window.open`。未注册该回调时
/// wry 会把 NewWindowRequested 标记为已处理却不做任何导航，表现为点击完全
/// 无反应。这里一律 Deny 新窗口，改为让调用方窗口自己导航到目标地址：授权页
/// 与调用方窗口同属一个 WebView2 实例、同一个数据目录，授权完成重定向回来时
/// 写入的 Cookie 立刻可见，也不存在白屏与弹窗关不掉的收尾问题。
///
/// 通道窗口（`channel-{platform}`）与登录窗口（`login-{platform}`）共用这一份
/// 实现，避免两处逻辑漂移。
pub fn with_same_window_navigation<'a, R, M>(
    builder: WebviewWindowBuilder<'a, R, M>,
    app: AppHandle<R>,
    label: String,
) -> WebviewWindowBuilder<'a, R, M>
where
    R: Runtime,
    M: Manager<R>,
{
    builder.on_new_window(move |new_url, _features| {
        // 只放行 web 地址：`javascript:` / `file:` 之类的 scheme 交给页面
        // 自己导航等于让页面拿 URL 直接执行脚本，一律拒绝。
        if !matches!(new_url.scheme(), "http" | "https") {
            eprintln!("[webview-nav] 拒绝非 http(s) 弹窗: {}", new_url);
            return NewWindowResponse::Deny;
        }
        let target = new_url.to_string();
        let app_for_nav = app.clone();
        let label_for_nav = label.clone();
        // 回调由 wry 通过 `dispatch_handler` 投递到消息循环执行，此时
        // NewWindowRequested 的 deferral 尚未 Complete。这里换一个线程发
        // eval，消息排队在本次回调之后处理，避免在事件回调内重入导航。
        std::thread::spawn(move || {
            let Some(window) = app_for_nav.get_webview_window(&label_for_nav) else {
                eprintln!("[webview-nav] 窗口已销毁，跳过跳转: {}", target);
                return;
            };
            match navigate_script(&target) {
                Ok(script) => {
                    if let Err(error) = window.eval(&script) {
                        eprintln!("[webview-nav] 当前窗口跳转失败: {}", error);
                    }
                }
                Err(error) => eprintln!("[webview-nav] {}", error),
            }
        });
        NewWindowResponse::Deny
    })
}

/// 打开（或复用）通道窗口。窗口已被用户关闭时 `get_webview_window` 返回 `None`，
/// 这里直接重建，不缓存任何已销毁的 handle。
///
/// 命中已存在窗口时只做 show/focus，不做任何导航或注入：需要干净文档的调用方
/// 必须先关窗再开窗（知乎的 `close_leftover_window` 就是干这个的）。
pub fn open_channel_window(
    app: &AppHandle,
    platform: &str,
    editor_url: &str,
) -> Result<WebviewWindow, String> {
    open_channel_window_with_visibility(app, platform, editor_url, true)
}

/// 同 [`open_channel_window`]，但可以指定新窗口是否立即可见。
///
/// 传 `false` 时窗口以隐藏状态创建，调用方可以先用 `window.url()` 静默探测登录态：
/// 探测成功就全程不打扰用户，失败再 `show()` 让用户就地登录。适合「Cookie 大概率
/// 仍然有效」的场景，避免每次同步都让窗口在屏幕上闪一下。
///
/// **注意**：隐藏窗口仍会加载页面并执行脚本，`window.url()` / `window.title()` 照常
/// 可读，因此轮询通道不受影响；但 WebView2 对隐藏窗口有定时器节流，依赖
/// `setInterval` 心跳的适配器应保持「终态写 URL hash」这类非定时器通道。
pub fn open_channel_window_with_visibility(
    app: &AppHandle,
    platform: &str,
    editor_url: &str,
    visible: bool,
) -> Result<WebviewWindow, String> {
    let platform = platform.trim();
    if is_placeholder_platform(platform) {
        return Err(format!("通道平台标识为空或占位值: {}", platform));
    }

    let label = channel_window_label(platform);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(existing);
    }

    let url = Url::parse(editor_url.trim())
        .map_err(|error| format!("通道编辑页地址非法: {}", error))?;
    // 显式绑定数据目录：登录态跨重启保留全靠它，不能依赖框架的隐式默认值。
    let data_dir = webview_data_dir(app)?;
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(format!("{} - 发布通道", platform))
        .inner_size(1000.0, 760.0)
        .data_directory(data_dir)
        .visible(visible);
    with_same_window_navigation(builder, app.clone(), label.clone())
        // `target="_blank"` 的登录链接（简书的微信 / QQ 登录就是这种）在这一层
        // 就被改写为当前窗口打开，不依赖 `.on_new_window` 的事后补救。
        .initialization_script(SAME_WINDOW_LINK_SCRIPT)
        // Tauri 的原生窗口标题不会自动跟随 `document.title`。通道适配器用
        // 「页面把状态写进 `document.title`、Rust 侧读 `window.title()`」这一条
        // 纯 DOM 回传路径，绕开所有 IPC 与权限问题，因此必须把文档标题同步到
        // 原生标题，否则 `window.title()` 永远只会返回建窗时的固定标题。
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(&title);
        })
        .build()
        .map_err(|error| format!("创建通道窗口失败: {}", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_label_is_prefixed_and_trimmed() {
        assert_eq!(channel_window_label("zhihu"), "channel-zhihu");
        assert_eq!(channel_window_label("  zhihu  "), "channel-zhihu");
        assert_eq!(CHANNEL_WINDOW_PREFIX, "channel-");
    }

    #[test]
    fn placeholder_detection_rejects_common_junk() {
        for value in ["", "0", "-", "placeholder", "UNKNOWN", "null", "none"] {
            assert!(is_placeholder_platform(value), "应识别占位平台: {}", value);
            assert!(is_placeholder_post_id(value), "应识别占位标识: {}", value);
        }
        assert!(!is_placeholder_platform("zhihu"));
        assert!(!is_placeholder_post_id("123456"));
    }

    #[test]
    fn navigate_script_wraps_url_as_json_literal() {
        let target = "https://zhihu.com/path?a=1&b=\"x\"";
        let script = navigate_script(target).unwrap();
        assert!(script.starts_with("window.location.href = "));
        assert!(script.contains("https://zhihu.com/path?a=1&b=\\\"x\\\""));
        assert!(script.ends_with(';'));
    }

    #[test]
    fn same_window_link_script_intercepts_blank_targets() {
        let script = SAME_WINDOW_LINK_SCRIPT;
        assert!(script.contains("addEventListener('click'"));
        assert!(script.contains("preventDefault"));
        assert!(script.contains("window.location.href"));
        assert!(script.contains("_blank"));
        // 改写兜底：页面加载完成后把已存在的 _blank 链接就地改成 _self。
        assert!(script.contains("setAttribute('target', '_self')"));
        // 幂等守卫，避免每次导航重复注册监听。
        assert!(script.contains("__OPENPUB_SAME_WINDOW_LINKS__"));
    }

    /// 回归：头条登录页底部的「微信 / QQ 登录」走 `window.open(url)`，不是
    /// `target="_blank"` 链接。未劫持时它会落到 NewWindowRequested，wry 把
    /// 弹窗标记为已处理却不导航，表现就是「点了完全没反应」。
    #[test]
    fn same_window_link_script_hijacks_window_open() {
        let script = SAME_WINDOW_LINK_SCRIPT;
        // 必须保存原生实现，并在调用时改走当前窗口导航。
        assert!(script.contains("var nativeOpen = window.open;"));
        assert!(script.contains("window.open = hijackedOpen;"));
        assert!(script.contains("return window;"));
        // 只接管 http(s)，其余 scheme 交回原生实现由 Rust 侧拒绝。
        assert!(script.contains("isHttpUrl"));
        // 登录 SDK 可能在运行中重新覆盖 `window.open`，必须定期复查。
        assert!(script.contains("ensureOpenHijacked();"));
    }
}