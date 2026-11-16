# Tasks

> 范围锁定：本变更只修真机暴露的阻断（格式转换、51CTO 标签、抖音登录竞态、阿里云 / 开源中国 / 百家号 / 网易号登录态、知乎 Draft.js 正文兼容性），不新增平台。`tencentcloud.rs` / `aliyun.rs` / `oschina.rs` 原生吃 Markdown，**不得**改动。
>
> 第 4~7 节（阿里云、开源中国、百家号、网易号）为真机回归中新暴露的阻断，追加进本变更而非另开变更：它们与抖音同属「HttpOnly 凭据读不到导致窗口卡死」，修法共用同一套 Rust 侧 Cookie 轮询基建，拆开会让同一根因分五次归档。仅改动 `login.rs` 与 `registry.ts` 的注释，未改各平台适配器的正文逻辑。
>
> 这五次重复暴露说明「HttpOnly 凭据」不是个例而是通用问题：接 Cookie 凭据的平台都可能中招。因此第 4~7 节共享同一套五步流程——加白名单 → 补跨域 URL → 写专属判据 → 加键名日志 → 加单测，并在 spec 中提为独立 Requirement。

## 1. 基础设施与 Markdown 转换器

- [x] 1.1 `src-tauri/Cargo.toml` 引入 `comrak = { version = "0.59", default-features = false }`
- [x] 1.2 重写 `platforms/shared/markdown.rs` 的 `to_html_content`：`comrak::Options`（非 `ComrakOptions`）、`render.r#unsafe`（非 `unsafe_`）、开启 table / strikethrough / autolink / tasklist、开启 `render.hardbreaks`、输出 `trim` 且空内容回落 `<p></p>`
- [x] 1.3 重写 `markdown.rs` 单测：代码块、列表、表格、单换行硬换行、原始 HTML 透传、空输入兜底
- [x] 1.4 更新 `juejin.rs` 中关于 `to_html_content` 的旧断言
- [x] 1.5 `wechat.rs` 清洗逻辑迁到 HTML 层：剔除 `script` / `style` / `iframe` / `object` / `embed`，`<img>` → `[图片]`，`<a>` 收敛为纯文本；同步更新受影响的单测

## 2. 平台通道修正

- [x] 2.1 `fifty_one_cto.rs` 标签归一化：删除 `replace(',', " ")`，按 `[,，、]` 拆分、去空、保序去重、逐标签截断 20 字符、内部空格转 `-`、逗号拼接、总长超 100 从尾部丢标签并在成功消息中提示
- [x] 2.2 `fifty_one_cto.rs` 正文改传 `markdown::to_html_content`
- [x] 2.3 `cnblogs.rs` 正文改为 `xml_escape(&markdown::to_html_content(&request.content))`
- [x] 2.4 `webview/xiaohongshu.rs` 正文传参前剥离 Markdown 符号（`#`、`*`、反引号、`[]()` 链接语法），保留纯文本与换行
- [x] 2.5 `webview/douyin.rs` 的 `long_article` 改传 `markdown::to_html_content`，并在注入前 `eprintln!` 打印前 100 字符

## 3. 抖音登录竞态

- [x] 3.1 `ensure_login_window` 增加稳定性判定：Cookie 判据 `passport_assist_user`（值非空）连续 3 次（间隔 500ms）命中才算登录成功
- [x] 3.2 `watch_login` / `recover_login` 同步同一稳定性判定，避免首次命中即关窗
- [x] 3.3 注入脚本把 `status_code === 8` 归到 `AUTH:` 通道，而非 `FAIL:` 文本
- [x] 3.4 `run_fetch_phase` 命中 `ChannelError::Auth` 时：显窗 → 等待重新登录（稳定性判定）→ 回到长文编辑页 → 重发一次 draft 请求，仅一次
- [x] 3.5 删除 `is_logged_in_url`：URL 在 SPA 未登录时同样渲染 `creator-micro` 骨架，不能作为登录判据
- [x] 3.6 `auth_cookie_snapshot` 改走 `cookies_for_url` 读 HttpOnly Cookie，并要求 `passport_assist_user` 值非空（空值 Cookie 不算登录）
- [x] 3.7 注入悬浮状态条 `__openpub_login_banner`：每轮轮询「重注入 + 重设状态」，`pointer-events:none` 不挡页面操作
- [x] 3.8 登录成功先切绿色悬浮条并停留 800ms 再关窗；认证失败分支一律保留窗口不关
- [x] 3.9 `login-failed` 事件新增 `definitive` 字段：仅「窗口被关且始终无登录 Cookie」才允许前端降级为未登录

## 4. 阿里云登录态

- [x] 4.1 `login.rs` 的 `COOKIE_POLL_PLATFORMS` 加入 `"aliyun"`，启用 Rust 侧 Cookie 轮询
- [x] 4.2 `supplemental_cookie_urls("aliyun")` 补 `https://login.aliyun.com/` 与 `https://account.aliyun.com/`（票据在统一登录域，`c_csrf` 在业务域，三域都要查）
- [x] 4.3 新增 `aliyun_logged_in_cookie` 专属判据，精确匹配 `login_aliyunid_ticket` / `login_aliyunid` / `aliyun_session`，值非空且非 `0` / `null`
- [x] 4.4 `has_really_logged_in` 对 `aliyun` 路由到专属判据（不再走 `AUTH_COOKIE_HINTS` 子串匹配）
- [x] 4.5 `spawn_cookie_poller` 每轮为阿里云单独打印 `[login] 阿里云轮询 keys=[...]`（仅键名，不含值）
- [x] 4.6 单测：`cookie_with_aliyun_ticket_is_logged_in`、`aliyun_anonymous_cookies_are_not_logged_in`、`supplemental_urls_cover_aliyun_login_and_account_domains`；并修正 `cookie_poll_only_enabled_for_strict_verdict_platforms` 的白名单断言
- [x] 4.7 `registry.ts` 中阿里云的过时注释改为「走 Rust 侧 Cookie 轮询」，删除「URL 跳转 / DOM 探针判定」的表述
- [x] 4.8 真机回归：阿里云扫码登录 → 状态正确、窗口正常收尾

> 4.3 刻意**不收**裸 `t`：它是阿里系通用埋点字段，匿名访问即有值。纳入判据会让轮询在窗口打开第一秒误判、秒关窗口并存下匿名凭据。通用白名单的 `token` 子串同样会放行匿名 `_tb_token_`，两条误判路径已各有一条单测钉住。

## 5. 开源中国登录态

- [x] 5.1 `login.rs` 的 `COOKIE_POLL_PLATFORMS` 加入 `"oschina"`，启用 Rust 侧 Cookie 轮询
- [x] 5.2 `supplemental_cookie_urls("oschina")` 补 `https://my.oschina.net/` 与 `https://www.oschina.net/`（票据常在个人中心子域下发，发布接口 `apiv1.oschina.net` 靠它换会话）
- [x] 5.3 新增 `oschina_logged_in_cookie` 专属判据，精确匹配 `oscid` / `oschina_new_user` / `oschina`，值非空且非 `0` / `null`
- [x] 5.4 `has_really_logged_in` 对 `oschina` 路由到专属判据
- [x] 5.5 轮询日志抽成通用函数 `cookie_key_log_label` / `cookie_key_names`（阿里云与开源中国共用，避免复制排序去空逻辑）
- [x] 5.6 单测：`cookie_with_oschina_oscid_is_logged_in`、`oschina_anonymous_cookies_are_not_logged_in`、`supplemental_urls_cover_oschina_profile_subdomain`、`cookie_key_logging_covers_platforms_with_unstable_key_names`
- [x] 5.7 真机回归：开源中国扫码登录 → 状态正确、窗口正常收尾

> 5.3 的 `oschina` / `oschina_new_user` 属**弱信号**（匿名访问也可能被下发）。真机回归未观察到「窗口一打开就秒关」，故保留；若后续出现秒关，按常量上方注释删掉这两个键、只留 `oscid`，不得反过来放宽判据。

## 6. 百家号登录态

- [x] 6.1 `login.rs` 的 `COOKIE_POLL_PLATFORMS` 加入 `"baijiahao"`，启用 Rust 侧 Cookie 轮询
- [x] 6.2 `supplemental_cookie_urls("baijiahao")` 补 `https://passport.baidu.com/` 与 `https://baijiahao.baidu.com/`（票据由百度通行证下发）
- [x] 6.3 新增 `baijiahao_logged_in_cookie` 专属判据，精确匹配 `BDUSS` / `BDUSS_BFESS` / `STOKEN` / `PTOKEN`，值非空且非 `0` / `null`
- [x] 6.4 `has_really_logged_in` 对 `baijiahao` 路由到专属判据
- [x] 6.5 `cookie_key_log_label` 增加 `baijiahao → 百家号`，逐轮打印 `[login] 百家号轮询 keys=[...]`
- [x] 6.6 单测：`cookie_with_baijiahao_bduss_is_logged_in`、`baijiahao_anonymous_cookies_are_not_logged_in`、`supplemental_urls_cover_baijiahao_passport_domain`，并更新白名单与日志映射断言
- [x] 6.7 真机回归：百家号登录 → 状态正确、窗口正常收尾

> 6.3 刻意**不收** `BAIDUID` / `BAIDUID_BFESS` / `BIDUPSID` / `PSTM` / `H_PS_PSSID`：这些是百度匿名访客就带的全量标识与统计字段。其中 `BIDUPSID` 含 `sid` 子串，若有人往 `AUTH_COOKIE_HINTS` 加 `sid`，百家号会立刻误判，故必须走专属判据。

## 7. 网易号登录态

- [x] 7.1 `login.rs` 的 `COOKIE_POLL_PLATFORMS` 加入 `"netease"`，启用 Rust 侧 Cookie 轮询
- [x] 7.2 `supplemental_cookie_urls("netease")` 补 `https://reg.163.com/`、`https://passport.163.com/` 与 `https://mp.163.com/`（业务入口在 mp.163.com，票据可能落在 163 通行证子域）
- [x] 7.3 新增 `netease_logged_in_cookie` 专属判据：主认 `NTES_YD_SESS`（mp.163.com 业务域票据），兜底 `NTES_SESS` / `NTES_SESSID`（passport.163.com 主站键名），值非空且非 `0` / `null`
- [x] 7.4 `has_really_logged_in` 对 `netease` 路由到专属判据
- [x] 7.5 `cookie_key_log_label` 增加 `netease → 网易号`，逐轮打印 `[login] 网易号轮询 keys=[...]`（仅键名，不含值）
- [x] 7.6 单测：`cookie_with_netease_session_is_logged_in`、`netease_anonymous_cookies_are_not_logged_in`、`netease_anonymous_key_names_are_never_predicates`、`netease_needs_dedicated_predicate_because_generic_hints_match`、`supplemental_urls_cover_netease_passport_domain`
- [x] 7.7 真机回归：网易号扫码登录 → 状态正确、窗口正常收尾

> 7.3 的键名归属经真机日志两次校正，最终定案：主信号是 `NTES_YD_SESS`（登录前无、登录后有，Cookie 总长 220 → 512 字节，差值即该票据）。`NTES_SESS` 是网易通行证主站的会话键名，与网易号业务域票据不是一回事，**只作兜底**，不得因「名字更像 session」提到前面。
> 7.3 刻意**不收** `P_INFO` / `S_INFO` / `NTESwebSI` / `_ntes_nuid` / `_antanalysis_s_id` / `_ntes_origin_from`：`P_INFO` 等为匿名即带值的埋点字段，首轮接入时曾因此秒关窗口；`S_INFO` 虽在登录后才出现，但与 `P_INFO` 同族、语义不明确，同样不进判据。

## 8. 知乎 Draft.js 正文兼容性

> 真机现象：知乎草稿只剩「标题 + 结尾」，中间大段丢失。根因不在发布链路，而在正文格式——知乎正文存于 Draft.js 的 Immutable 数据结构，对 `<table>` / `<pre><code>` 这类结构标签解析失败后整段丢弃，只保留零散 `<p>`。修法分两步：先收紧 HTML 兼容性，再把注入方式换成模拟粘贴。

- [x] 8.1 `shared/markdown.rs` 新增 `to_zhihu_html_content`：在通用 `to_html_content` 之上做降级，不改动 `to_html_content` 本身，其余平台输出零变化
- [x] 8.2 代码块降级：`<pre><code>` 整体替换为 `<blockquote><p>…</p></blockquote>`，行间用 `<br />` 维系，彻底消除 `<pre>` / `<code>` 标签
- [x] 8.3 表格扁平化：逐行提取 `<td>` / `<th>` 文本，单元格用 ` | ` 连接、行间 `<br />`，整体包成一个 `<p>`，输出中不再出现 `<table>` / `<tr>` / `<td>` / `<thead>`
- [x] 8.4 标签白名单：仅保留 `p` / `br` / `strong` / `b` / `em` / `i` / `a` / `h1`-`h6` / `ul` / `ol` / `li` / `blockquote` / `code` / `hr`，其余标签剥壳留文本；`<a>` 只保留 `href`
- [x] 8.5 `webview/zhihu.rs` 正文改用 `to_zhihu_html_content`，注入优先级改为「模拟 `paste`（`DataTransfer` + `ClipboardEvent`）→ `insertHTML` → `innerHTML` → 纯文本」，让 Draft.js 走自己的粘贴解析链路
- [x] 8.6 单测：代码块降级、代码块换行保留、表格扁平化、基础标签保留、非法标签剥壳留文本、`<img>` 丢弃、特殊字符转义、空内容兜底；`zhihu.rs` 补模拟粘贴断言
- [x] 8.7 真机回归：知乎草稿内容布局确认通过（含代码块与表格的长文不再中途丢失）

> 8.2 / 8.3 的取舍：知乎本来就不支持代码高亮与原生表格，降级后**信息不丢、只是形态变朴素**。`<img>` 有意排除在白名单外——知乎图片需走上传接口，粘贴外链图成功率不可控，一张失败可能拖垮整个块；若后续要支持，须单独走上传链路而不是放宽白名单。
> 8.5 用合成 `ClipboardEvent` 而非「写系统剪贴板 + 模拟 Ctrl+V」：效果等价（Draft.js 认的都是 `onPaste`），但不污染用户剪贴板、不依赖按键模拟，且这套方案已在简书通道真机验证。

## 9. 验收

- [x] 9.1 `cargo test --lib` 全绿（459 条：原 436 + 阿里云 3 + 开源中国 4 + 百家号 3 + 网易号 5 + 知乎 8）
- [x] 9.2 `cargo build` 通过（含 `comrak` 新依赖）
- [x] 9.3 真机回归（重点链路）：抖音、51CTO 完整链路通过
- [x] 9.4 真机回归（阿里云）：扫码登录通过
- [x] 9.5 真机回归（开源中国）：扫码登录通过
- [x] 9.6 真机回归（百家号、网易号）：登录通过
- [x] 9.7 真机回归（知乎）：草稿内容布局通过
- [x] 9.8 真机回归（其余平台）：头条、博客园、微信、掘金
- [x] 9.9 归档：合并 delta spec 进 `openspec/specs/` 后移入 `changes/archive/`

> 9.3 ~ 9.8 已覆盖本变更改动最深的五处（抖音登录判据与悬浮条、51CTO 标签归一化、阿里云 / 开源中国 / 百家号 / 网易号的 Cookie 轮询、知乎的 Draft.js 正文降级、其余平台的富文本与微信清洗）。
> 9.8 验证了富文本 `<pre>`/`<table>` 与微信 `<img>`/`<a>` 清洗，
> 与已通过部分无交叉依赖，失败不影响其结论。头条同样走 Cookie 凭据，
> 若真机再报「探针 auth=none、窗口卡死」，按第 4~7 节同一套五步流程接入即可。

---

## 10. v1.0.6 合并：关于窗口 Windows 版本识别 + 阿里云 payload 契约锁

> 范围：Rust 侧修正「关于」窗口 Windows 版本识别；阿里云 payload 经查证与 v1.0.0 一致，不改 payload，仅加契约锁测试并归档查证结论。前端零改动。

### 10.1 查证与结论归档

- [x] 10.1.1 逐行比对当前 `aliyun.rs` 与 v1.0.0 快照 `.aliyun_v100.txt`：payload 字段名、正文格式（Markdown 原文）、`eprintln!` 打印均一致或更完善
- [x] 10.1.2 确认修复指令中的 `markdown` / `categoryId` / `tagIds` 结构在 v1.0.0 中从未存在，结论「不恢复、防回归」写入 proposal/design

### 10.2 Windows 版本识别（Rust 侧）

- [x] 10.2.1 `src-tauri/src/lib.rs` 新增纯函数 `display_windows_major_for_build(reported_major, build)`：`build >= 22000` 返回 11，否则返回 `reported_major`
- [x] 10.2.2 新增纯函数 `windows_edition_name(product_type)`：把 `PRODUCT_PROFESSIONAL`/`PRODUCT_BUSINESS` 映射「专业版」、`PRODUCT_CORE`/`PRODUCT_HOME_*` 映射「家庭版」系、`PRODUCT_ENTERPRISE*` 映射「企业版」、`PRODUCT_EDUCATION*` 映射「教育版」、`PRODUCT_PRO_WORKSTATION*` 映射「专业工作站版」、`PRODUCT_ULTIMATE*` 映射「旗舰版」，未收录返回 `None`
- [x] 10.2.3 `os_version_string()` 改为：`RtlGetVersion` 失败仍走 `fallback_os_version()`；成功时用 `display_windows_major_for_build` 定主版本，调 `GetProductInfo` 取产品类型，有版本名输出 `Windows 11 专业版 (Build 26300)`、无版本名输出 `Windows 11 (Build 26300)`
- [x] 10.2.4 单测：22000 边界、各版本常量映射、未知类型 `None`、`os_version_string()` 输出包含 "Windows" 且非空

### 10.3 阿里云 payload 契约锁（仅测试）

- [x] 10.3.1 `src-tauri/src/platforms/aliyun.rs` 测试区新增 `payload_field_names_match_v100_editor_contract`：断言 payload 键集合恰为 `{title, content, contentRender, format, aid, productTags, freeTierVOS}`，且 `markdown` / `categoryId` / `tagIds` 三键 `is_none()`
- [x] 10.3.2 测试注释写明：契约来自平台编辑器真实请求（与 v1.0.0 一致）；若平台未来改结构，按新抓包改 `build_draft_payload` 并同步更新本测试

### 10.4 验收

- [x] 10.4.1 `cargo test --lib` 全绿（479 条，新增 3 条）
- [x] 10.4.2 `cargo build` 通过
- [x] 10.4.3 逻辑真机校验：`os_version_string()` 实际输出 `Windows 11 专业版 (Build 26300)`
- [ ] 10.4.4 真机回归（用户执行）：关于窗口底部应显示「系统 Windows 11 专业版 (Build 26300)」或同形态文本
- [ ] 10.4.5 真机回归（用户执行）：清空阿里云原有空草稿，重新同步一次，网页端打开草稿标题与正文正常