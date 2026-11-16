# Design

## Decision 1：引入 `comrak`，推翻 v1.0.0 的「不引入 Markdown 渲染库」非目标

`v1.0.0-omni-release` 的 Non-Goals 明确写着「不引入 Markdown 渲染库（沿用共享的『转义 + 每行 `<p>`』实现）」。真机回归证明该实现无法满足 10 个平台的真实排版需求：代码块、列表、表格全部丢失。

- **结论**：引入 `comrak = { version = "0.59", default-features = false }`。关闭默认特性可砍掉 CLI 与 `syntect` 语法高亮，避免无关的体积与编译开销。该版本 `rust-version = 1.89`，本项目工具链为 1.99.1，不撞 MSRV。
- **API 说明**：0.59 的类型是 `comrak::Options`（`ComrakOptions` 是旧名），透传原始 HTML 的字段是 `options.render.r#unsafe`（raw identifier，不是 `unsafe_`）。
- **代价**：`markdown.rs` 的 4 条单测、`juejin.rs` 的 1 条断言、`wechat.rs` 的链接断言需重写；产物体积增加约 400KB。

## Decision 2：保留两条旧语义，避免「修完格式更乱」

- **空输入兜底**：`platform-shared-utils` 现有 Scenario 要求「空输入产出单个空 `<p>`，绝不为空串」。comrak 对空输入返回 `""`，因此转换器在 `trim()` 后为空时回落 `<p></p>`。
- **单换行即换行**：旧实现「每个非空行一个 `<p>`」，comrak 默认把连续两行并成同一段落（`\n` 在 HTML 里折叠成空格）。中文作者的书写习惯是「单换行 = 换行」，若不处理，用户会看到段落全部粘连。故开启 `options.render.hardbreaks = true`（输出 `<br />`）。
- **输出 trim**：comrak 输出带尾随换行，转换器统一 `trim()`，让所有调用方与断言拿到稳定字符串。

## Decision 3：外链图片差异化处理

- **微信公众号**：`<img>` 一律替换为 `[图片]` 文本。`unsafe = true` 后用户手写的原始 `<img>` 也会透传，因此清洗必须发生在 HTML 层，不能只在 Markdown 层剥 `![]()`。
- **其它平台**：保留 `<img>`，交由平台自身转存。若真机出现大面积裂图，再针对性做图床适配，不在本变更范围内。

## Decision 4：微信 `<a>` 收敛为纯文本

公众号正文不支持可点击外链，原实现刻意把 `[文档](url)` 保留为文本。`unsafe = true` 后 comrak 会产出 `<a href="...">文档</a>`，需在 HTML 层收敛为链接文字，否则草稿里会留一个点了没反应的死链。

## Decision 5：51CTO 标签归一化

根因不是「缺少截断」，而是 `article_form` 里的 `request.tags.replace(',', " ")`：把前端 `tags.join(",")` 的分隔符抹掉后，51CTO 收到的「单个标签」就是全部标签拼成的长串，必然触发 20 字上限。

- 删除该 `replace`，按 `[,，、]` 拆分；
- trim、去空、保序去重；
- 每个标签按字符（非字节）截断至 20，复用 `shared/truncate.rs` 的思路，但**不带**「...(已截断)」后缀（后缀会被当成标签正文）；
- 标签内部空格转 `-`，防止被服务端按空格二次切分；
- 用 `,` 重新拼接；总长超过 100 字符时从尾部丢标签，并在成功消息里追加「标签已精简」提示，避免用户以为丢数据。
- **100 是保守假设值**，未由抓包证实；发生精简时会有提示，可据真机反馈调整。

## Decision 6：抖音 `status_code=8` 走 `AUTH` 通道并自动重试

`status_code=8` 在语义上就是「未登录」，不应走 `FAIL:` 字符串通道让 Rust 侧解析文本。脚本里单独判定 `status_code === 8` 写 `AUTH:`，Rust 侧命中已有的 `ChannelError::Auth` 分支，重试闭环挂在那里：

1. `window.show()` + `set_focus()`；
2. 复用 `ensure_login_window`（其稳定性判定已修好）等待用户完成扫码；
3. `ensure_editor_page` 回到长文编辑页；
4. `run_fetch_phase` 重跑一次（其入口已有 `CLEAR_SESSION_SCRIPT` 清 `sessionStorage`，不会重放旧状态）；
5. 只重试一次，仍失败则如实报错。

## Decision 7：抖音 `long_article` 改传 HTML（带实测回退口）

按变更指令改传 `to_html_content`，并在注入前打印前 100 字符。风险已知：若抖音后端把 `long_article` 当 Markdown 渲染，传 HTML 会得到字面标签，比现状更糟。因此该改动**必须**与日志一起落地，真机若出现字面 `<p>`，回退为 Markdown 原文只需改一行。

## Decision 8：小红书正文纯文本化

小红书图文笔记的正文最终走 `stores.draft.desc`，注入脚本内部还会 `stripHtml`。传 Markdown 原文会让 `#`、`*` 直接出现在笔记描述里。按指令在传参前剥离 Markdown 符号、保留换行与纯文本。

已知副作用：`extractImages` 依赖 `<img>` 标签，传纯文本时永远取不到图，会一直走 `generateCover` 兜底封面——这与现状（传 Markdown）行为一致，不是回归。若后续要让正文配图生效，需改为传 HTML，届时单独评估。

## Decision 9：Webview 平台登录判定必须依赖真实凭据（Cookie），严禁依赖 URL 或 DOM 探针

`v1.0.0` 与 Decision 6 的登录判据都是「URL 落在 `creator-micro` 路由」。真机证明这条判据结构性错误：

- **SPA 假阳性**：抖音创作者中心未登录时同样会渲染 `/creator-micro/` 路由骨架，URL 判据返回 `true`。窗口因此在 1.5 秒内自动关闭，用户还没扫码就被判定「已登录」，随后 `draft` 接口必回 `status_code=8`。用户同时看到「窗口秒关 + 提示已登录 + 发布说没登录」三句互相矛盾的话。
- **HttpOnly 盲区**：真正的 Session Cookie（`passport_assist_user`）是 HttpOnly 的，页面内 `document.cookie` 读不到。任何「注入脚本读 Cookie 再回传」的 DOM 探针方案都无法用于登录判定。
- **判据与官方一致**：原始插件 `checkAuth` 就是读 `.douyin.com` 的 `passport_assist_user`，与本次实现同源。

**结论**：登录态只认 Rust 侧 `window.cookies_for_url()` 读出的底层 Cookie 容器，且值必须非空（匿名访客也可能拿到同名的空值 Cookie，只看键名会把「未登录」判成「已登录」）。`is_logged_in_url` 直接删除而不是标 `deprecated`——留着它，下一个接手的人就会继续调用它。

**推广到所有 Webview 平台**：知乎、简书、小红书、头条、思否的登录判定同样不得依赖 URL 或 DOM 探针。各平台的凭据键名不同，但读取路径统一走 `cookies_for_url`。

## Decision 10：未登录绝不关窗，并把判定结果实时反馈到页面上

Decision 9 修好判据后，还剩下「用户判断不了窗口里是什么状态」这一层体验问题，因此配套三条约束：

- **未登录绝不关窗**：`ensure_login_window` / `watch_login` / `recover_login` 以及发布路径上两条认证失败分支，全部改为「显窗 + 聚焦 + 保留」，直到 300 秒超时。窗口是用户唯一的补救入口（扫码、过滑块验证），关掉等于把入口一起收走。
- **悬浮状态条**：页面顶部固定 `__openpub_login_banner`，橙色=等待登录、绿色=已登录。每轮轮询「重注入 + 重设状态」，脚本幂等不会叠条。`pointer-events:none` 是必需的——固定栏若参与命中测试会挡住抖音页面顶部自己的按钮。
- **先反馈再关窗**：登录确认后先切绿条、停留 800ms 再 `window.close()`，否则窗口在扫码成功瞬间消失，用户无从确认。
- **失败分级**：`login-failed` 事件带 `definitive` 字段。超时（窗口仍开着、用户可能正在扫码）为非确定失败，前端不得据此降级登录态，否则下一次同步会被 `runPublish` 静默跳过；只有「窗口被关且始终没有登录 Cookie」才是确定失败，允许前端标「未登录」。

实现细节坑：注入脚本含 CSS 颜色（`"#15803d"`）时，`r#"..."#` 会被字符串里的 `"#` 提前截断，编译报 `prefix 'b45309' is unknown`，必须用 `r##"..."##`。

## Decision 11：阿里云接入 Rust 侧 Cookie 轮询，判据只认 SSO 票据

真机现象：扫码登录后页面跳回 `www.aliyun.com`，DOM 探针能识别登录特征（`hits=1`），但 `auth=none`——鉴权 Cookie 是 `HttpOnly`，页面内 `document.cookie` 读不到，探针永远等不到确认信号，窗口卡在登录页。与抖音、51CTO 同一根因。

- **接入轮询**：`COOKIE_POLL_PLATFORMS` 加入 `"aliyun"`。该白名单只收「判定足够严格」的平台，每新增一个都必须同时补一条专属判据，否则等于把宽松的键名白名单塞进轮询。
- **三域并查**：阿里云横跨 `login.aliyun.com`（统一登录入口）、`account.aliyun.com`（账号页）、`developer.aliyun.com`（业务域）。SSO 票据下发在统一登录域，`c_csrf` 却下发在业务域的 host-only Cookie 上，只查任意一侧都会漏。`cookies_for_url` 不接受通配域名，只能按具体 URL 逐个查。
- **判据**：`aliyun_logged_in_cookie` 精确匹配 `login_aliyunid_ticket` / `login_aliyunid` / `aliyun_session`，要求值非空且非 `0` / `null`。`has_really_logged_in` 对 `aliyun` 路由到它，不再走 `AUTH_COOKIE_HINTS` 子串匹配。

**刻意不收裸 `t`**（指令里提到它「典型」，但真机上它不可用）：

- `t` 是阿里巴巴系通用埋点字段，**匿名访问即有值**（时间戳类）。纳入判据会让轮询在窗口打开第 1 秒就 `passed=true`，立刻抓 Cookie 关窗并存下一份匿名凭据，前端显示「已登录」而发布必失败——正是 Decision 9 / 10 花整轮修掉的「秒关窗口 + 误报已登录」。
- 通用白名单也有同类漏洞：`_tb_token_` 含子串 `token`，匿名 Cookie 会被 `auth_cookie_key` 放行。已用单测把这条误判路径钉住（`assert_eq!(auth_cookie_key(anonymous).as_deref(), Some("_tb_token_"))`）。
- 若真机日志显示登录后只剩 `t`，正确做法是先定位票据落在哪个域（对照浏览器 F12 的 Cookie 面板），再补域名或键名；直接塞 `t` 只是把误判请回来。

**日志先行**：`spawn_cookie_poller` 每轮为阿里云单独打印 `[login] 阿里云轮询 keys=[...]`（仅键名）。键名清单能区分「域名没覆盖到」与「键名判断错」两种失败，比任何猜测都快。

## Decision 12：开源中国接入 Rust 侧 Cookie 轮询，并抽出共用日志函数

真机现象：扫码登录后页面已回到 `oschina.net` 且渲染了登录特征（`hits=3`），但探针读到 `auth=none`。`oscid` 是 `HttpOnly` 会话票据，页面内读不到，探针死锁。与 Decision 11 完全同因，处理方式照搬。

- **接入轮询**：`COOKIE_POLL_PLATFORMS` 加入 `"oschina"`。
- **两域并查**：登录入口在 `www.oschina.net/home/login`，但票据常在个人中心 `my.oschina.net` 下发——发布接口 `apiv1.oschina.net` 正是拿这份票据换会话。只查登录入口会读不到 `oscid`，于是 `supplemental_cookie_urls` 补 `my.oschina.net` 与 `www.oschina.net`。
- **判据**：`oschina_logged_in_cookie` 精确匹配 `oscid` / `oschina_new_user` / `oschina`，值非空且非 `0` / `null`；`has_really_logged_in` 对 `oschina` 路由到它。
- **弱信号留了退路**：`oschina_new_user` 与 `oschina` 从字面看是「新老访客 / 站点标记」，匿名首访也可能被下发。真机回归未观察到「窗口一打开就秒关」，故保留；若后续出现秒关，按常量上方注释删掉这两个键、只留 `oscid`，**不得反过来放宽判据**。

**日志抽成通用函数，而不是第三次复制粘贴**：阿里云那段的「排序、去空、join」逻辑被抽为 `cookie_key_log_label` / `cookie_key_names`，平台到展示名的映射集中在 `match` 里。新增平台只需加一行 `match` 分支，日志格式只有一处定义。

**去重是按完整 URL 字符串，不是按域名**：`with_supplemental_urls` 比对的是 URL 全串，登录页 `www.oschina.net/home/login` 与补充域 `www.oschina.net/` 路径不同，两者都会保留。这不是 bug——同域多读一次 Cookie 只是多一次查询，`assemble_cookie_string` 按同名保留最后一个值，不产生重复键。写单测时曾误断言「去重后只剩 2 个」，实际是 3 个，已按真实行为修正。

## Decision 13：百家号接入轮询，并把「HttpOnly 凭据平台」固化为五步流程

百家号与阿里云、开源中国同因：登录票据 `BDUSS` 是百度通行证的 `HttpOnly` Cookie（域 `.baidu.com`），页面内 `document.cookie` 读不到，DOM 探针只能看到 `auth=none`。

- **接入轮询**：`COOKIE_POLL_PLATFORMS` 加入 `"baijiahao"`。
- **两域并查**：业务入口 `baijiahao.baidu.com`，票据由 `passport.baidu.com` 下发，两处都查。业务域那一项虽然与窗口默认 URL 同域、属冗余查询，但保留：若真机显示只有查 passport 域才拿到票据，说明它是该子域的 host-only Cookie，这个信息值得留在配置里而不是删掉后再踩一次。
- **判据**：`baijiahao_logged_in_cookie` 精确匹配 `BDUSS` / `BDUSS_BFESS` / `STOKEN` / `PTOKEN`，值非空且非 `0` / `null`；`has_really_logged_in` 对 `baijiahao` 路由到它。`BDUSS_BFESS` 是 SameSite / 分区 Cookie 改造后的变体，`STOKEN` / `PTOKEN` 是同套通行证下发的接口票据。

**刻意不收百度匿名字段**：`BAIDUID` / `BAIDUID_BFESS` / `BIDUPSID` / `PSTM` / `H_PS_PSSID` 是匿名访客就带的全量标识与统计值。`BIDUPSID` 尤其危险——它含 `sid` 子串，若有人往 `AUTH_COOKIE_HINTS` 加 `sid`，百家号会立刻误判。这与 Decision 11 里 `_tb_token_` 含 `token` 是同一类陷阱：**子串白名单对「匿名也发同名 Cookie」的站点天然不可靠**。

**五步流程固化**。同一根因已重复五次（抖音、阿里云、开源中国、百家号、网易号），说明它不是个例而是接 Cookie 凭据平台的通病。后续（头条等）一律按这五步走，顺序不可跳：

1. 加 `COOKIE_POLL_PLATFORMS` 白名单
2. 补 `supplemental_cookie_urls` 跨域地址（票据所在域 ≠ 业务域时必须补）
3. 写专属判据 `{platform}_logged_in_cookie`，精确匹配强键名 + 值非空非 `0` / `null`
4. 在 `cookie_key_log_label` 加一行 `match` 分支，逐轮打印键名清单（只打键名）
5. 加单测：正向票据、匿名拒绝、跨域地址覆盖

**为什么顺序不可跳**：先加白名单再写判据，中间那一瞬 `has_really_logged_in` 会落到通用白名单，轮询可能在登录页上就 `passed=true` 秒关窗口。五步是一个原子动作，拆开提交就会留下一段会误判的中间态。

## Decision 14：网易号的主信号是 `NTES_YD_SESS`，不是通行证的 `NTES_SESS`

网易号（`mp.163.com`）与前述四家同因：扫码登录后页面已渲染登录特征，DOM 探针却读到 `auth=none`，因为票据是 `HttpOnly`。

- **接入轮询**：`COOKIE_POLL_PLATFORMS` 加入 `"netease"`。
- **三域并查**：业务入口 `mp.163.com`，票据可能下发在 `reg.163.com` 或 `passport.163.com`（163 通行证体系），三处都查。
- **判据**：`netease_logged_in_cookie` 主认 `NTES_YD_SESS`，兜底 `NTES_SESS` / `NTES_SESSID`，值非空且非 `0` / `null`。

**键名归属经两次真机日志校正**，这是本变更里唯一一处「判据名字错了两次」的案例，值得单独立案：

1. 首轮按语感选了 `NTES_SESS` / `NTES_SESS2` / `P_INFO` / `S_INFO`。真机日志立刻证伪：`P_INFO` 等字段匿名访客就带值，轮询在扫码前就 `passed=true`。
2. 收紧为 `NTES_SESS` / `NTES_SESSID` 后仍然判不准。日志对比登录前后：Cookie 总长从 220 字节变成 512 字节，新增的键是 `NTES_YD_SESS`——它才是 `mp.163.com` 自己的会话票据。`NTES_SESS` 是 `passport.163.com` 主站的会话键名，两者不是一回事。

**保留 `NTES_SESS` 作兜底，但必须排在 `NTES_YD_SESS` 之后**：某些登录流程下主站会话可能同时出现，删掉会让这类账号判不出登录态；但顺序不能反——`NTES_SESS` 名字更像 session，若排第一，后续维护者会误以为它是网易号的主信号，下一次校正键名时又踩回同一个坑。代码注释里因此写死了「不得因名字更像就提到前面」这条禁令。

**`S_INFO` 的处置值得单独记一笔**：它在日志里也是「登录后才出现」，看起来像强信号，但与 `P_INFO` 同族、语义不明确（匿名链路同样会写）。最终决定不进判据——宁可少一个信号，也不要多一次误判。这条判断依据与 Decision 11 / 13 一致：**看未登录时有没有值，不是名字像不像**。

**回归锁**：单测 `netease_anonymous_key_names_are_never_predicates` 把 `P_INFO` / `S_INFO` / `NTESwebSI` / `_ntes_nuid` / `_ntes_origin_from` / `_antanalysis_s_id` / `NEGID` / `NTES_P_UTID` / `NTES_SESS2` 全部列为弱键名并断言不得进入判据，同时断言 `NTES_YD_SESS` / `NTES_SESS` / `NTES_SESSID` 必须仍在判据里。任何人顺手把弱键名加回、或误删主信号，单测会立刻失败。

## Decision 15：知乎正文走「HTML 降级 + 模拟粘贴」，而不是继续用通用 HTML 硬灌

真机现象是**草稿只剩「标题 + 结尾」，中间大段丢失**。最初怀疑是发布链路（草稿 API、自动保存时序），实际根因在正文格式本身：

- 知乎正文存在 Draft.js 的 Immutable 数据结构里，编辑器对 HTML 的兼容性极挑剔。
- 通用 `to_html_content` 输出的 `<table>`、`<pre><code>` 在 `insertHTML` / 直接写 DOM 时被解析失败，**整段被丢弃**，只有最简单的 `<p>` 段落被保留——于是看到的就是不连贯的残片。

**第一步：共享工具内新增知乎专属降级，而不是在适配器里自己拼 HTML**

`to_zhihu_html_content` 在通用转换结果之上做三层处理，`to_html_content` 本身零改动，其余平台输出完全不受影响：

1. 代码块 → `<blockquote><p>…</p></blockquote>`，行间 `<br />`。知乎对引用块的兼容性远高于 `<pre>`。
2. 表格 → 逐行 `单元格 | 单元格` 的纯文本，行间 `<br />`，整体包一个 `<p>`。原生表格在知乎基本必丢。
3. 标签白名单收口到 `p / br / strong / b / em / i / a / h1-h6 / ul / ol / li / blockquote / code / hr`，其余剥壳留文本；`<a>` 只留 `href`。

**为什么放在 `shared/markdown.rs` 而不是 `zhihu.rs`**：`platform-shared-utils` 的既有 Requirement 明确禁止适配器各自实现 Markdown 解析。降级属于「同一份 HTML 的不同兼容性视图」，只有放在共享工具里，才既能复用 comrak 的解析结果，又不违反那条约束。因此 spec 里同步把「跨平台转换结果一致」的表述补上知乎这一处**有意的例外**。

**第二步：注入方式从「硬灌 DOM」换成「模拟粘贴」**

注入优先级改为：模拟 `paste`（`DataTransfer` + `ClipboardEvent`）→ `insertHTML` → `innerHTML` → 纯文本。

- 关键理由是 Draft.js 的 `onPaste` 是它自己的解析入口，走这条路等于把 HTML 交给知乎自己的解析器处理，成功率最高；`insertHTML` 是绕过编辑器状态机的旁路，正是「大段丢失」的现场。
- 用**合成 `ClipboardEvent`** 而不是「写系统剪贴板 + 模拟 Ctrl+V」：Draft.js 认的都是 `onPaste` 事件对象，两者对编辑器等价；但合成事件不污染用户剪贴板、不依赖按键模拟，也不会在无焦点窗口里静默失败。这套写法在简书通道已真机验证，直接复用。
- 保留 `bodyFilled` 探针校验与 1500ms 自动保存等待：粘贴失败仍会逐级降级，且绝不把「没填进去」误判成「填好了」直接点发布。

**有意排除 `<img>`**：知乎图片必须走上传接口，粘贴外链图成功率不可控，一张失败可能拖垮整个块。要支持图片应新增上传链路，而不是把 `<img>` 加回白名单——后者只会把「丢一张图」变成「丢一段文」。

**取舍已接受**：代码块失去等宽排版与语法高亮，表格失去列对齐。知乎本就不支持这两项，降级后**信息不丢、只是形态变朴素**，这比「整段消失」是严格更优的结果。

**回归锁**：8 条 markdown 单测分别钉住代码块降级、代码块换行、表格扁平化、基础标签保留、非法标签剥壳留文本、`<img>` 丢弃、特殊字符转义、空内容兜底；`zhihu.rs` 的单测断言脚本必须同时包含 `new DataTransfer()` / `new ClipboardEvent('paste'` 与三级兜底，防止后续维护者把它简化回纯 `insertHTML`。

---

## v1.0.6 合并：关于窗口 Windows 版本识别 + 阿里云 payload 契约锁

### Decision 16：Build 号判定 Windows 11，映射留在 Rust

- Windows 的兼容性 shim 让 `RtlGetVersion` 对 Win11 仍报 `10.0`，这是平台契约不是 bug。判定逻辑（`build >= 22000 → 主版本 11`）放在 Rust 侧 `os_version_string()` 内，输出已是人话字符串，前端 `AboutDialog.tsx` 原样渲染，零改动。
- 边界：22000 是 Windows 11 RTM 的起始 Build；Server 2025 之类不受影响（本应用桌面端不跑 Server 场景）。

### Decision 17：版本名走 `GetProductInfo`，未知类型静默降级

- `windows-sys` 0.59 的 `Win32_System_SystemInformation` feature（已启用）含 `GetProductInfo` 与 `PRODUCT_*` 常量，**不新增依赖、不加 feature**。
- `GetProductInfo(10, 0, 0, 0, &mut product)` 用真实报告的 Major/Minor，调用即得当前系统的产品类型。
- 映射只收桌面常见类型：专业、家庭、企业、教育、专业工作站、旗舰；未收录的返回 `None`，输出退回 `Windows 11 (Build xxxxx)`——宁可少显示，不可显示错。
- **真机事实（Build 26300）**：`GetProductInfo(10, 0)` 返回 SKU 139（`PRODUCT_PRO_CHINA`），该常量在 windows-sys 0.59 中未收录，按 winnt.h 值以本地常量补进映射并锁进单测。

### Decision 18：输出格式带 Build 号

- `Windows 11 专业版 (Build 26300)` / `Windows 11 (Build 26300)`。旧格式 `Windows 10.0.19045` 在 Win10 上变为 `Windows 10 (Build 19045)`，信息量等价。
- 纯函数拆分：`display_windows_major_for_build(reported_major, build)` 与 `windows_edition_name(product_type)` 均为可单测纯函数。

### Decision 19：阿里云契约锁测试断言「字段名集合」而非顺序

- `serde_json` 默认 `Map` 是 `BTreeMap`（键按字典序），断言顺序会随机地错。测试断言键集合相等，并对 `markdown` / `categoryId` / `tagIds` 三键逐个断言 `is_none()`，作为显式回归信号。
- 未来如果平台真把端点换成 `categoryId`/`tagIds` 结构，正确动作是按新抓包改 `build_draft_payload` 并同步更新该测试。

### Risks (v1.0.6)

- 「关于」窗口的版本名依赖 `GetProductInfo` 在目标系统上的返回；未收录 SKU 显示为无版本名（可接受，不误导）。中国市场的 138/139 两个专业版变体已收录并真机验证。
- 真机回归（打开应用看关于窗口、阿里云重新同步草稿）需要用户操作，本变更以 `cargo test` + 真机回归任务项交接。