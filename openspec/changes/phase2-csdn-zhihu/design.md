# Design

## Context

动机见 `proposal.md` 的 Why。以下是影响方案选择的现状事实：

- `platforms/mod.rs` 已定义 `PublishRequest`（含 `title` / `content` / `existing_id` / `is_update` / `publish_status` / `existing_published`）与 `PublishResult`（含 `post_id` / `edit_url` / `message` / `failure_class` / `operation` / `is_published`），`dispatch` 用 `match platform` 路由。接入新平台不需要改请求/结果结构。
- `platforms/juejin.rs` 内含四个本应共享的私有实现：`read_cookie`、`truncate_chars`、`escape_html` / `to_html_content`。`cnblogs.rs` 也自带一份 `truncate_chars`。两份实现已经同构，说明该上提。
- `lib.rs` 的 `platform_publish` 在 `result.success == true` 且 `post_id` 非占位符时才写 sync_map。这个条件无法表达「建稿成功但发布失败」。
- `packages/core/src/platforms/registry.ts` 的 `available` 与 `ADAPTER_IMPLEMENTED_IDS` 由人工维护，`auditRegistry()` 做双向一致性校验（已实现必可用、可用必有实现）。
- `App.tsx` 第 606 行用 `selectedPlatforms.has("juejin")` 硬编码渲染「掘金仅支持保存草稿」提示，注册表没有表达该能力的字段。
- 掘金草稿更新的字段契约已实测锁定（`mark_content` / `content` / `category_id` / `edit_type` / `draft_id` + `id`），并已归档进 `platform-adapters` 主 spec。CSDN 与知乎的契约尚未验证。

## Goals / Non-Goals

**Goals:**

- 让 CSDN 与知乎接入后，`dispatch` 之外的核心路由逻辑（`existing_published` 决策、占位 ID 过滤、映射回填、错因归类）零改动
- 建立一份可被三个平台复用的 Cookie / Markdown / CSRF / 截断实现，且掘金既有行为不变
- 用注册表能力位替代前端平台 id 硬编码
- 让知乎两段式流程的中间态（草稿已建、发布失败）不丢稿、不重稿

**Non-Goals:**

- 不引入 Markdown 渲染库（保持与掘金已实测输出一致）
- 不做分类 / 标签 / 话题的 UI 映射，沿用掘金现状（平台默认值或空数组）
- 不做图片上传与图床
- 不改 `platform_publish` 的命令签名与前端调用参数
- 不做微信公众号、思否、腾讯云、阿里云

## Decisions

### Decision 1: 共享工具按关注点拆为目录模块

在 `platforms/shared/` 下按关注点拆分：`cookie.rs`（键值提取）、`csrf.rs`（token 提取 + URL-decode）、`markdown.rs`（转义与段落包装）、`truncate.rs`（按字符截断）。

**理由**：四个关注点各自有独立契约与独立测试面，拆开后单测可按契约命名。备选方案是把它们塞进 `platforms/mod.rs`，但 `mod.rs` 已是路由与类型定义的公共门面，再加工具函数会让「契约定义」与「工具实现」混在一起，后续每接一个平台都要改 `mod.rs`。

`cnblogs.rs` 的私有 `truncate_chars` 一并替换为共享实现——它当前与掘金版本逻辑相同，替换是纯等价重构。

### Decision 2: 契约验证先于实现，结论写回 design

每个平台都先完成浏览器抓包、把端点与字段契约写进本文件的 Decision 表格，再开始写适配器代码。

**理由**：掘金 Phase 1 的教训是 `err_no: 2` 只在真机请求后才暴露，字段名错一个字母就整链失败且错误信息不指向具体字段。把契约先钉死，能让适配器的单测直接断言字段名，而不是靠联调时试错。

**备选**：先写一版代码再靠联调修正。放弃原因是掘金那次花了多轮才对齐，且每轮都要真机请求，反馈周期长。

### Decision 3: CSDN 复用掘金的 `existing_published` 单一路由维度

> 现状：CSDN 已因网关强制签名降级为不交付，本 Decision 仅作为 `csdn.rs` 储备代码的设计说明，见 Decision 10。

CSDN 适配器不引入新的路由输入，`resolve_url` 结构与掘金同构：`Create` → 草稿保存端点；`Update` + `existing_published = false` → 草稿更新端点（即使 `publish_status = true`）；`Update` + `existing_published = true` → 文章更新端点。

**理由**：Phase 1 已把「未发布永远不走线上接口」作为强约束并用单测双锁，这条约束对所有「草稿与线上是两个对象」的平台都成立。复用它意味着 CSDN 的路由单测可以照抄掘金的结构，只是换 URL 常量。

**备选**：为 CSDN 引入独立的 `article_status` 字段。放弃原因是它会引入第二个路由维度，两个维度冲突时（如 `article_status=published` 但映射说未发布）需要仲裁规则，而这类歧义正是 Phase 1 花力气消除的。

### Decision 4: 知乎两段式在适配器内部串行，对外仍是单一结果

`zhihu::publish` 内部按顺序执行：解析 Cookie 与 XSRF → 建稿（或更新草稿）→ 若 `publish_status = true` 且目标需上线，再发一次发布动作。对 `dispatch` 只返回一个 `PublishResult`。

**理由**：两段式是知乎的平台特性，不应泄漏到前端或路由层。前端只关心「这次同步成功没有、文章在哪」。

**备选**：暴露 `create_draft` 与 `publish_article` 两个独立命令由前端编排。放弃原因是前端将不得不按平台 id 判断是否需要第二步，正好违背「前端不感知平台实现」这一既有约束，也会让 sync_map 的写入时机分散到前端。

### Decision 5: 部分成功复用 `post_id` 携带草稿 ID，映射写入条件改为「ID 可用」

`PublishResult` 结构不变。知乎建稿成功、发布失败时返回 `success: false` + `failure_class: platform_error`，但 `post_id` 与 `edit_url` 填入已建草稿。

`lib.rs` 的 sync_map 写入条件从 `result.success` 改为「`post_id` 非空且非占位符」，`published` 一律取 `result.is_published`。

**理由**：失败结果带 ID 是既有结构已支持的表达（`post_id` 是普通字符串字段，不因 `success` 而为空），无需改类型；改写入条件即可让两段式的中间态落进映射，下次同步自动走 update 分支，不会重稿。

**备选**：让适配器自己写 sync_map。放弃原因是适配器不该知道 `article_id`（本地文章 UUID）——那是前端概念，会破坏分层，也会让适配器单测需要伪造 store。

### Decision 6: 注册表新增发布能力位

`PlatformDefinition` 增加 `publishCapability: "live" | "draft_only"`。掘金为 `draft_only`，CSDN 与知乎为 `live`，博客园为 `live`。

**理由**：当前「掘金仅支持草稿」提示是前端硬编码平台 id，接第二个仅支持草稿的平台时必须复制一份判断。能力位把该判断收敛回注册表，与既有 `available` 同源。

**备选**：前端维护 `DRAFT_ONLY_IDS` 数组。放弃原因是又一处人工双写，且 `auditRegistry()` 无法校验它。

### Decision 7: 知乎需要额外签名时降级而非硬撑

若契约验证发现知乎接口需要掘金与 CSDN 之外的签名参数（如 `x-zse-96`），且无法用 Cookie 内已有信息复现，则知乎适配器返回 `not_implemented`、注册表保持 `available: false`，本次变更仅交付 CSDN。

**理由**：逆向签名算法的成本与风险远超本阶段目标，且硬编码一个会随时失效的签名比明确告知「暂不支持」更糟。这条降级不影响 CSDN 交付，也不影响已定的任务拆分结构。

### Decision 8: Markdown 转换不引入渲染库

继续使用「转义 HTML 特殊字符 + 每行包 `<p>`」的轻量实现，仅把它上提为共享模块。

**理由**：掘金已用该实现实测通过，平台侧接受度已被证明。引入 `pulldown-cmark` 会改变输出（标题变 `<h1>`、列表变 `<ul>`），等于在重构的同时改动已通过链路的行为，一旦平台侧渲染异常将无法区分是重构引入还是渲染差异导致。

**权衡**：输出保真度低（`# 标题` 不会变成 `<h1>`）。这是既有已知局限，与本次变更无关，留待后续独立变更处理。

### Decision 9: 启动白屏按「窗口层 + HTML 层」两个来源分别解决

启动期的白屏实际有两个独立来源，必须分别处理，只修其中一个仍会闪现：

- **窗口层**：Tauri 创建窗口后到 webview 首次绘制之间存在一段系统默认背景色，深浅主题下表现为黑屏或白屏。解法是在 `tauri.conf.json` 的窗口配置里显式声明 `backgroundColor: "#09090b"` 与 `theme: "Dark"`，让窗口自第一帧起就是深色，不依赖 webview 内容。
- **HTML 层**：`index.html` 的 `<div id="root"></div>` 在 JS bundle 执行前是空的，这段空白期没有任何内容可画。解法是把加载动画以内联原生 HTML + CSS 写进 `index.html`（深色背景 + 绿色呼吸圆点 + 「正在唤醒工作台...」），不依赖 React、不依赖任何构建产物。

移除时机由 React 侧控制：`App.tsx` 新增 `onReady` 回调，挂在文章列表加载完成之后（而不是组件挂载时）触发，避免「动画已移除但内容还没渲染」造成的二次空屏；`main.tsx` 的 `dismissBootSplash()` 先加 `is-hidden` 类走 CSS 过渡，再在 `transitionend` 时 `remove()`，并以 400ms 超时兜底防止过渡事件丢失，另有 6 秒硬超时防止极端情况下动画永久残留。

**理由**：两个来源的修复位置完全不同——窗口层在配置里，HTML 层在静态文件里，React 侧只负责「何时移除」。把它们混为一谈会导致「改了一处以为修好了，换个机器又复现」。

**为什么不当作 bug 回归**：这是产品体验层的既定设计而非临时补丁。后续任何人改动 `index.html`、`main.tsx` 的启动流程或 `tauri.conf.json` 的窗口配置，都应视为可能回退该体验，需要重新验证首帧颜色与动画移除时机。

### Decision 10: CSDN 因阿里云网关强制签名而降级，草稿同步暂不交付

> **已被 Decision 13 取代**：密钥已取得并获明确授权，CSDN 已恢复上线。本节保留作为决策过程与风险论证的记录。

真机验证 `POST https://bizapi.csdn.net/blog-console-api/v3/mdeditor/saveArticle` 返回 `{"message":"X-Ca-Key is not exist"}`。这是阿里云 API 网关的标准报错：网关在业务层之前校验 `x-ca-key` / `x-ca-nonce` / `x-ca-signature` / `x-ca-signature-headers` 四件套（HMAC-SHA256 签名），缺任一即拒绝，请求根本到不了 CSDN 业务层。

因此 CSDN 降级为 `available: false`，并从 `ADAPTER_IMPLEMENTED_IDS` 移除；`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` 同步移除 `"csdn"`，避免出现「能存 Cookie 却发不出去」的假登录态。

`platforms/csdn.rs` 与 `mod.rs` 的 `dispatch` 分支**保留不删**，作为未来若找到稳定签名方案时的储备；当前该分支不会被前端触发（`available: false` + 登录白名单已关闭）。

**排除的方案 A（逆向签名）**：从 CSDN 前端 JS 提取 SecretKey 与签名串拼法，在 Rust 侧实现 HMAC-SHA256。放弃原因有三：一是 SecretKey 内嵌于前端 JS，CSDN 轮换即整体失效且无预警（表现为突然 401），维护成本长期高于收益；二是属于逆向密钥的灰色地带；三是「先不带签名、命中 Invalid Signature 再降级到签名」的混合方案也不采纳——它本质仍要维护 SecretKey，只是把崩盘时间往后推，不解决根本问题。

**理由**：草稿同步是真需求，但用一份随时失效的逆向密钥去换它，会把「明确的暂不支持」变成「随时可能静默失败」。宁可明确告知不可用，也不制造脆弱链路。这条降级只影响 CSDN，不影响知乎交付与已定的任务结构。

**回退条件**：若后续发现 CSDN 提供官方开放接口（含正式 API Key 申请渠道），可重新评估接入；届时复用已保留的 `csdn.rs` 契约代码，仅补签名或改端点。

### Decision 11: 知乎因全站双层动态签名而降级，不交付

抓包确认知乎全站采用**双层动态签名**：发布端点 `api/v4/content/publish` 同时携带 `x-zse-96` 与 `x-zst-81`，且登录轮询、免责声明、草稿历史等四个请求**全部带签名**。签名值每次请求动态变化，需实时计算，其中 `x-zst-81` 疑似由 WASM 产出。

因此知乎降级为 `available: false`，并从 `ADAPTER_IMPLEMENTED_IDS` 移除；`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` 同步移除 `"zhihu"`，避免出现「能存 Cookie 却发不出去」的假登录态。知乎适配器代码**未开始实现**（不存在 `zhihu.rs`），故无储备代码可留。

**明确排除：逆向签名**。理由是双层签名里至少一层依赖 WASM，逆向成本远超 CSDN 的单层 HMAC；签名随请求动态变化，即使复现也需持续跟随知乎前端更新，属长期不可控技术债。这与 Decision 10 对 CSDN 的排除理由同源，不重复论证。

**这是第二个因签名而降级的平台，因此确立平台选型前置规则**：接入任何新平台前，**第一个问题必须是「是否需要反爬签名」**，而不是「接口能不能调通」。判定顺序为：① 有无官方开放 API（含正式密钥申请渠道）→ ② 是否纯 Cookie 无签名 → ③ 是否需要签名。需要签名的一律判为高难度、暂不推荐，不做逆向。

**回退条件**：若知乎开放官方 API 或出现免签名的公开端点，可重新评估；届时从 Decision 4 的两段式设计开始实现。

### Decision 12: 微信公众号无签名，按「阶段 A」交付纯文字草稿同步

抓包确认公众号后台的草稿接口**没有任何动态签名头**（无 `x-zse-*` / `x-ca-*` / `X-Signature`），鉴权完全依赖 Cookie + URL 里的 `token`，因此按 Decision 10/11 确立的前置规则，它属于「② 纯 Cookie 无签名」一类，可以进入实现。

**范围限定为阶段 A：只同步纯文字草稿**。图片上传、封面设置、群发/发布均不在本期。

**不走一键登录**。`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` **不加入 `"wechat"`**，`requiredCookieKeys` 保持空数组。原因有三：① 登录窗口刚打开时微信就会种下游客 Cookie，空数组会让「窗口刚开就已登录成功」的误判直接触发自动关闭；② 若改成填具体键名，键名本身是猜的，猜错则窗口永远关不掉；③ 即使扫码成功也拿不到 `token`（token 在后台 URL 里，不在 Cookie 里），一键登录拿不到可用凭据。因此前端对该平台只提供「手动填 Cookie + 手动填 token」。

**凭据双源，校验语义必须写清楚**。注册表新增 `requiredConfigKeys: ["token"]`，语义是「`wechat_config` 对象里 `token` 键非空」，**与 Cookie 无关**，不复用 `requiredCookieKeys`（复用它会把 token 校验绑到登录窗口的成功判定上，正是上面第 ① 条的坑）。Rust 侧对应新增 `shared/cookie.rs` 的 `load_config_field`，与 Cookie 读取完全分离。

**字段严格对齐抓包，不猜、不抄第三方**。关键事实：

- `sub` / `type=77` / `token` / `lang` 在 **URL 查询串**里，不在 body；
- 索引字段一律用 `0` 系列：`title0` / `content0` / `digest0` / `author0`；
- 抓包是多图文（`articlenum=2`、`count=2`），单篇必须改成 `articlenum=1`、`count=1`，并且**不传任何 `1` 系列字段**（`title1` 等）；
- 抓包中**不存在** `is_multi` / `multi_item`，不臆造这两个字段名；
- 其余静态字段取自抓包实际出现的：`fingerprint` / `random` / `data_seq` / `req` / `remind_flag` / `is_auto_type_saving` / `save_type` / `isneedsave`。

**content0 生成前先剥 Markdown 图片语法**。`![alt](url)` 会被替换为 `[图片]` 占位，然后再走 `to_html_content`。否则 Markdown 图片在 HTML 转换里留下 `![]()` 垃圾文本，直接进草稿正文。

**digest0 先纯文本化再截断**。先剥离 Markdown 标记（`#`、`**`、反引号、链接/图片语法）与换行，再取前 120 字。摘要里出现 `#`、`**` 这类符号观感很差，纯文本化成本低，直接做。

**成功判定只看响应体 `ret`**。公众号接口出错时仍返回 HTTP 200，因此不能以状态码判成功：`ret == 0` 为成功，非 0 取 `errmsg` 归类 `platform_error`；响应体不是 JSON 一律判失败。`errmsg` 必须与 `ret` 取自同一层对象（`base_resp` 结构下两者都在嵌套层里）。

**`publish_status=true` 与 `existing_published=true` 返回 `NotImplemented`**。公众号文章一旦发布就不能再改正文，这两条分支必须显式拒绝，不能静默降级为更新草稿。

**create 未验证时的处理方式：不做兜底**。create 请求的字段组合尚未在真实环境中跑通。若失败，正确动作是**开发者重新抓一次 create 请求**来校正字段，而不是让用户手动建稿再填 `AppMsgId`。「手动建稿 + 填 AppMsgId」的兜底方案**默认不实现**。代码侧只做一件事：失败时完整打印请求字段与原始响应，让差异一眼可见。

**回退条件**：若 create 端点最终确认不可用（如需要前端侧额外计算的一次性参数），则把 `available` 改回 `false`、从 `ADAPTER_IMPLEMENTED_IDS` 移除，`wechat.rs` 保留为储备代码；已存的 Cookie/token 不受影响。

### Decision 13: CSDN 恢复上线，签名密钥本地计算（明确授权的规则例外）

CSDN 是「签名预判」规则的**唯一例外**，已获明确授权覆盖 Decision 10 的降级结论。

**密钥来源**：`X_CA_KEY = 203803574` 与 HMAC-SHA256 的 `SECRET_KEY` 均取自 CSDN 前端签名实现的源码分析结果（对应源码中的 `hb` 变量），不是官方下发的开放 API 密钥。这两个值硬编码在 `platforms/csdn.rs` 的两个常量里，代码中不含任何运行期密钥分发逻辑。

**签名实现**：`url_path` / `build_string_to_sign` / `compute_signature` / `new_nonce` / `sign_headers` 全部为纯函数，`publish` 只负责组装。StringToSign 为阿里云网关的 7 段格式：

```
POST\n*/*\n\napplication/json\n\nx-ca-key:203803574\nx-ca-nonce:{uuid-v4}\n{path}
```

GET 请求的 Content-Type 段为空，因此是连续两个空行。`{path}` 只取 URL 的路径部分，不含域名与 query。

**必须成对维护的常量**：签名里用的 Accept / Content-Type 必须与真实请求头完全一致。代码里它们被抽成 `ACCEPT_ANY` / `CONTENT_TYPE_JSON` 两个常量，签名与请求共用，避免两处漂移。这一点有单测 `signature_is_sensitive_to_content_type_drift` 锁死——若有人把请求头改成 `application/json;charset=UTF-8` 而没同步改签名，测试会立刻失败。

**已知失效风险（接受）**：密钥内嵌于前端 JS，CSDN 轮换时本应用会静默失效，表现为 `401` 或 `X-Ca-Key is not exist`。恢复动作是重新取一次密钥、只改 `csdn.rs` 的 `SECRET_KEY` 常量。这不是回归 bug，是这条路径的固有维护成本，也是 Decision 10 当初拒绝它的核心理由——现在以「已授权、风险已知」的方式接受。

**规则同步**：`.trae/rules/project_rules.md` 的签名预判条款已改写，加入「唯一例外：CSDN」小节，并明确例外范围仅限 CSDN 一家，知乎的双层签名与后续新平台仍按默认规则判定。

**恢复项**：注册表 `available: true` 且加回 `ADAPTER_IMPLEMENTED_IDS`；`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` 加回 `"csdn"`（`settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS` 一直保留 `"csdn"`，无需改动）。

**直接发布：分阶段交付，绝不静默降级**。国内平台总方针是「先草稿、后发布」。草稿链路已于 2026-11-08 真机验证通过，直接发布分支据此落地。

**`saveArticle` 的 `status` 取值（2026-11-08 由列表接口状态字典反推确认）**：请求体 `status = 2` 为保存草稿，`status = 0` 为提交发布。**提交发布的实际效果为直接对外发布**（真机验证见下条），而非进入审核队列。必须区分两套状态取值——请求参数是数字（`0` / `2`），列表接口返回的是字符串（`draft` / `audit` / `all_v2`），二者不是同一字段，不得互相套用。此前源码注释里「`1` 表示直接发布」的说法已作废。

**发布成功的语义（2026-11-08 真机修正）**：此前假设 `status = 0` 会把文章送进审核队列、线上不可见，因此 `is_published` 写死为 `false`。真机结果推翻了该假设——勾选发布后，文章直接出现在 CSDN 后台「已发布」列表，「审核中/未通过」计数保持为 0，草稿箱计数同步减少。**结论：CSDN 对 `status = 0` + `pubStatus = "publish"` 的处理是直接对外发布（或审核秒过，等效于直接发布）。**

因此 `is_published` 改为直接取请求意图（`request.publish_status`），message 改为「已成功发布到 CSDN」。这个 `false` 曾经是系统里最危险的定时炸弹：映射记成未发布后，下一次未勾选覆盖的同步不会被 `resolve_url` 守卫拦截，会用 `status = 2` 把线上文章打回草稿箱，正是我们一直要防的静默降级。单测 `publish_success_marks_the_article_as_published` / `draft_success_is_not_marked_as_published` 双向锁死。

**`pubStatus` 与 v3 字段规范（2026-11-08 由 v3 `mdeditor/saveArticle` 抓包确认）**：v1 `postedit/saveArticle` 与 v3 `mdeditor/saveArticle` 是**两套不同的字段规范**，不可互相套用：v1 用 `read_type`（下划线）、`article_id`、`source: "pc_postedit"`，v3 用 `readType`（驼峰）、`source: "pc_mdeditor"`，且 v3 带 `pubStatus`（草稿为 `"draft"`）。本适配器只使用 v3，因此以 v3 规范为准。

**由此暴露的既有 bug**：代码此前一直发 `read_type`，而 v3 要求 `readType`。早期能拿到文章 ID `170288151` 推测是后端对未知字段宽容、有默认值兜底；校验收紧后该字段被当作未知字段忽略，`readType` 缺失即报错。这不是代码回归，是字段名从一开始就写错，只是此刻才暴露。

**body 字段对齐**：除 `readType` 外，v3 还要求 `level`、`original_link`、`authorized_status`、`not_auto_saved`、`source`、`cover_images`、`cover_type`、`is_new`、`vote_id`、`resource_id`、`pubStatus`、`creation_statement`、`creator_activity_id`。其中 `not_auto_saved` 是字符串 `"1"`（不是数字），`level` / `cover_type` / `is_new` / `vote_id` / `creation_statement` 是数字，`authorized_status` 是布尔。

**字段 diff 结论（2026-11-08）**：`build_body` 输出的 21 个字段与 v3 抓包逐字段一致、类型全部吻合，包含此前存疑的两处——`tags` / `categories` 官方即空字符串 `""`（非空数组），`cover_type: 1` 与 `cover_images: []` 官方就是该组合。抓包留档见 `docs/reference/csdn-v3-mdeditor-saveArticle-draft.curl.txt`。

**仍属推断的三处（抓包只有新建草稿，无对照物）**：① 更新主键字段名沿用 `id`（v3 更新包未抓到，失败模式为产生重复文章，可观测；但 4.6 真机更新后文章数未增加，已初步证实 `id` 被 v3 接受）；② `is_new` 只在 `Create` 分支发 `1`，`Update` 分支不带该字段——无条件发 `1` 有触发「新建而非更新」的风险，因此按 operation 收敛，更新时该字段的官方取值仍需抓包确认；③ 发布态 `status = 0` + `pubStatus = "publish"` 为推断。三项分别由 `tasks.md` 的 4.10 / 4.11 / 4.12 覆盖验证。

**请求头来源对齐**：`Origin` / `Referer` 改为 `https://editor.csdn.net`（v3 编辑器来源），与 v3 抓包一致。这两个头不参与 `x-ca-*` 签名（签名只含 `x-ca-key` 与 `x-ca-nonce`），因此改动不影响签名正确性。

**发布必须带标签（2026-11-08 真机确认）**：CSDN 对草稿与发布采用不同的校验尺度——草稿允许 `tags` 为空，发布则强制要求标签，缺失时返回 `{"code":400,"msg":"请设置文章标签"}`。该报错发生在业务层（签名已通过网关），因此它反证了两件事：签名路径正确、服务端确实按「要发布」的语义在做校验（`status = 0` 生效，不存在「200 但静默落草稿」的假成功路径）。当前 `PublishRequest` 没有标签字段，实现上发布分支填兜底常量 `DEFAULT_PUBLISH_TAG = "OpenPub"`，草稿分支保持 `""`；代价是每篇发布文章都会被打上该标签，接入标签输入 UI 后应改为透传用户标签。

**仍然保留的唯一拒绝**：`existing_published = true` 且 `publish_status = false`（目标是已发布文章、但用户没勾选覆盖）时，仍在**发出任何网络请求之前**拒绝。理由不变：把已发布文章的 id 传给 `saveArticle` 且 `status = 2`，最坏情况是把线上文章打回草稿（等于下线）。勾选了覆盖之后，该组合改为正常发布（`status = 0` + `id`）。

spec delta 已相应恢复「草稿保存 / 直接发布为两个动作」的意图，并把「未勾选覆盖的已发布目标必须拒绝」「勾选后走发布」写成可验证的 Scenario。

**仍不支持**：CSDN 发布后的审核结果回读（本应用不轮询审核状态，`is_published` 在发布后保持 `false` 直到下一次真机确认）。若签名通过后网关仍报错，优先怀疑三处：Content-Type 是否与签名一致、是否还缺 `x-ca-timestamp` 等额外签名头、密钥是否已轮换。

**密钥状态**：`SECRET_KEY` 已通过 DevTools 搜索逐字符核实为 `9znpamsyl2c7cdrr9sas0le9vbc3r6ba`（源码 `hb` 变量），源码注释里的「待核实」标注已删除。注意这仅代表**字面值正确**，签名是否被网关接受仍以真机 `saveArticle` 的返回为准。

**真机验收（已完成）**：2026-11-08 实机同步 CSDN 草稿成功，网关未报 `X-Ca-Key is not exist` / `SignatureDoesNotMatch`，返回真实文章 ID `170288151`，CSDN 后台草稿箱确认接收。这同时验证了三件事：签名算法复刻正确、StringToSign 的段数与空行约定正确、Cookie + CSRF 的鉴权路径可用。签名不再是 CSDN 的阻塞项。

## Risks / Trade-offs

- **CSDN / 知乎契约未验证，字段名错一个就静默失败** → 先完成 Decision 2 的抓包环节，把契约写进本文件后再写代码；联调阶段沿用掘金的「完整打印请求体与原始响应」策略，让字段错误在终端直接可见
- **知乎 `_xsrf` 是百分号编码的，漏解码表现为 403** → 共享 `csrf.rs` 统一做 URL-decode，并用单测锁定 `abc%3D%3D` → `abc==` 的转换
- **共享工具上提可能改坏已通过的掘金链路** → 上提作为独立步骤先行，完成后立即跑掘金既有 8 个单测；若失败则说明重构引入偏差，不继续后续平台工作
- **两段式中间态处理不当会产生重复草稿** → 建稿成功即写映射（Decision 5），并新增单测断言「部分成功后再次同步走 update 而非 create」
- **CSDN 风控对频率与 Referer 敏感** → 单篇文章单次同步只发必要请求，失败归类 `platform_error` 并附原始响应片段，不做自动重试
- **注册表 `available` 置真后用户会立刻看到可选** → 顺序上先完成适配器与单测，最后一步才改注册表，避免中间态出现「可选但不可用」

## Migration Plan

无数据迁移：凭据沿用既有 `{platform}_cookie` 键空间（DPAPI 加密），sync_map 结构不变（知乎部分成功仍写 `{id, published: false, bodyHash}`）。

部署顺序即任务顺序：契约验证 → 共享工具上提（掘金回归通过）→ CSDN → 知乎 → 注册表置真 → 全量测试。

回滚策略：注册表 `available` 与 `ADAPTER_IMPLEMENTED_IDS` 回退即可让用户侧立即看不到新平台；适配器代码可保留（`dispatch` 未注册则不会被调用）。共享工具为纯重构，回滚不影响数据。

## Open Questions

- CSDN 草稿保存是否需要额外的必填字段（如 `read_type`、`cover_image` 空值占位）—— 待抓包确认，只影响请求体常量，不影响任务拆分
- 知乎文章更新端点是否与建稿端点为同一路径的不同方法 —— 待抓包确认，只影响 `resolve_url` 的 URL 常量表