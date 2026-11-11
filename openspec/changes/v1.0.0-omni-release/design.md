# Design

## Context

本变更是 v1.0.0 的唯一总流程，归集 phase2（CSDN / 知乎）与 phase3（产品体验）。以下现状事实决定了方案选择：

- `platforms/mod.rs` 已定义 `PublishRequest`（`title` / `content` / `existing_id` / `is_update` / `publish_status` / `existing_published` / `tags`）与 `PublishResult`（`post_id` / `edit_url` / `message` / `failure_class` / `operation` / `is_published`），`dispatch` 用 `match platform` 路由。接入新平台不需要改请求/结果结构。
- `packages/core/src/platforms/registry.ts` 已登记 **18 个平台**，全部 `available: true`，与 `ADAPTER_IMPLEMENTED_IDS` 的 18 个 id 双向一致；`channelType` 取值为 `direct` / `webview`。
- `login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` 与 `settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS` 是新平台接入的双清单，漏改会导致「能存 Cookie 却发不出去」或导出时解密崩溃。两者当前均只含 `direct` 通道平台，`webview` 平台全部排除。
- Webview 能力有两条用途：**登录取 Cookie**（`login-{platform}` 窗口 + `capabilities/login.json`）与**在平台页面里执行发布脚本**（`channel-{platform}` 窗口 + `capabilities/webview-channel.json`），两者窗口分离、权限分离。
- `docs/index.mjs`（19 平台）与 `docs/index2.mjs`（18 平台，缺网易号）是参考实现（浏览器扩展形态），是本变更的契约事实来源。

## Goals / Non-Goals

**Goals:**

- 18 个国内平台全部出现在注册表，且每个平台都有明确通道、明确契约与明确验收标准
- `dispatch` 的路由逻辑、`PublishResult` 结构、`platform_publish` 命令签名零改动
- 需要运行时签名的平台**不逆向**，改为在其页面内执行其自身 JS
- 单个平台失败不影响其余平台；任何平台都不得把「发布」静默降级为「草稿」

**Non-Goals:**

- 不做海外平台（Dev.to / Hashnode / Medium），不在 v1.0.0 范围
- 不引入 Markdown 渲染库（沿用共享的「转义 + 每行 `<p>`」实现）
- 不做多账号矩阵、不做定时发布、不做审核状态轮询
- 不移植任何平台签名算法（唯一例外是已授权的 CSDN，已在基线中交付）
- 不接入抖音图文（`douyin-image`）与小红书长文笔记，二者留待后续版本

## 平台矩阵（契约事实来源：`docs/index.mjs` / `docs/index2.mjs`）

`index2.mjs` 覆盖 18 个平台，缺 `netease`；其余平台实现与 `index.mjs` 对齐，差异不构成阻塞。

| # | id | 名称 | 通道 | 主端点（神兵证据） | 鉴权 / 风险 | 发布能力 | 真机验证 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `cnblogs` | 博客园 | `direct` | MetaWeblog XML-RPC | Token（`credentialKind: metaweblog`），无签名 | 真实发布 | 通过 |
| 2 | `juejin` | 掘金 | `direct` | `content_api/v1/article_draft/create`、`article_draft/update`、`tag_api/v1/query_tag_list` | Cookie + `x-ware-csrf-token`（HEAD `user_api/v1/sys/token` 探针，须带 `x-secsdk-csrf-request: 1`，否则 404；取值 `split(",")[1]` 作为 `x-secsdk-csrf-token` 发业务请求） | 仅草稿 | 通过 |
| 3 | `csdn` | CSDN | `direct` | `bizapi.csdn.net/blog-console-api/v3/mdeditor/saveArticle` | Cookie + CSRF + 网关 HMAC（已授权例外） | 真实发布 | 通过 |
| 4 | `wechat` | 微信公众号 | `direct` | `mp.weixin.qq.com/cgi-bin/operate_appmsg`、`filetransfer` | Cookie + URL `token`（页面 HTML 正则）；`requiredConfigKeys: ["token"]` | 仅草稿（纯文字） | 通过 |
| 5 | `jianshu` | 简书 | `webview` | `www.jianshu.com/author/notes`、`/publicize`、`/settings/basic.json` | Cookie 完整仍 403 `code 9`：风控按客户端 TLS/HTTP 指纹拦截（见 Decision 15） | 仅草稿（`is_published` 恒 `false`，已确认为有意设计） | 通过 |
| 6 | `51cto` | 51CTO | `direct` | `blog.51cto.com/blogger/publish` + 存储上传 | Cookie，无签名 | 真实发布 | 通过 |
| 7 | `oschina` | 开源中国 | `direct` | `apiv1.oschina.net` 草稿/发布 | Cookie，无签名 | 真实发布 | 通过 |
| 8 | `tencentcloud` | 腾讯云 | `direct` | `cloud.tencent.com/developer/api/*` | Cookie，无签名（正文用 `<!--markdown-->` 包裹） | 真实发布 | 通过 |
| 9 | `aliyun` | 阿里云 | `direct` | `developer.aliyun.com/developer/api/article/publishArticle` | Cookie + `csrfHeaders` | 仅草稿 | 通过 |
| 10 | `weibo` | 新浪微博 | `direct` | `card.weibo.com/article/v5/editor` | Cookie + 页面 `config` JSON 正则（预检 `page_html`） | 真实发布 | 通过 |
| 11 | `baijiahao` | 百家号 | `direct` | `baijiahao.baidu.com/pcui/article/save?callback=bjhdraft`、`materialui/picture/uploadProxy` | Cookie，无签名（JSONP 解析） | 仅草稿 | 通过 |
| 12 | `netease` | 网易号 | `direct` | `mp.163.com` 表单 POST | Cookie + `wemediaId`/`realUserId` 预检（`navinfo`） | 仅草稿 | 通过 |
| 13 | `bilibili` | 哔哩哔哩 | `direct` | `api.bilibili.com`、`member.bilibili.com/platform/upload/text` | Cookie + `bili_jct`（Cookie 内自带 CSRF，预检 `nav`） | 仅草稿 | 通过 |
| 14 | `toutiao` | 今日头条 | `webview` | `mp.toutiao.com/mp/agw/*`、`ttwid/check/` | Rust 直发稳定返回 `7050 参数校验失败`，改判 `webview`（见 Decision 17） | 真实发布 | 通过 |
| 15 | `zhihu` | 知乎 | `webview` | `zhuanlan.zhihu.com/api/articles/drafts`、`api/v4/me`、`api.zhihu.com/images` | `x-zse-93` / `x-zse-96` / `x-zst-81` + `ZP` 函数，**全站双层签名** | 真实发布 | 通过 |
| 16 | `xiaohongshu` | 小红书(图文) | `webview` | `creator.xiaohongshu.com`、`api/galaxy/user/info` | `_webmsxyw` / `X-s-common`（`edith.xiaohongshu.com`） | 真实发布 | 通过 |
| 17 | `douyin` | 抖音 | `webview` | `creator.douyin.com/*` 注入脚本、`/web/api/media/aweme/draft` | `x-tt-verify-passport-decision` 等运行时校验 + 页面上下文特征 | 仅草稿 | 通过 |
| 18 | `segmentfault` | 思否 | `webview` | `segmentfault.com/gateway/{path}?keyv=&ivd=` | 页面内 `encrypt(JSON.stringify(...))` 生成加密串 | 真实发布 | **待验证（无测试账号）** |

> 表中 18 行与 `PLATFORM_REGISTRY` 一一对应。`douyin-image`（抖音图文）与小红书长文笔记不在 v1.0.0 范围，未登记。

通道计数：`direct` 12 个（其中 `juejin` / `wechat` / `weibo` / `netease` / `bilibili` 带预检）、`webview` 6 个（知乎、简书、思否、小红书图文、头条、抖音）。

发布能力计数：真实发布 10 个（博客园、CSDN、知乎、思否、小红书图文、51CTO、开源中国、腾讯云、微博、头条），仅草稿 8 个（掘金、微信公众号、阿里云、百家号、网易号、哔哩哔哩、抖音、简书）。

## Decisions

### Decision 1: 通道是注册表的一等能力位，而不是实现细节

`PlatformDefinition` 的 `channelType: "direct" | "webview"` 与既有 `credentialKind` **正交**：`credentialKind` 决定前端如何收集凭据（`cookie` / `webview` / `metaweblog`），`channelType` 决定后端如何执行请求。

**理由**：`credentialKind` 已经无法表达「凭据怎么收」与「请求怎么发」的差异。例如知乎是 `cookie` 凭据但必须 `webview` 执行；微博是 `cookie` 凭据但需要先 GET 一次页面抽 token。把它们混成一个枚举会迫使前端按平台 id 硬编码判断，正是 phase2 Decision 6 要消除的模式。

**为什么只有两种枚举**：`direct` / `webview` 对应两种**执行位置**——Rust 直发、平台页面内执行。博客园的 MetaWeblog 由 `credentialKind: "metaweblog"` 表达，执行位置仍是 Rust 直发，因此不单列 `channelType` 值。是否需要先取页面 token / CSRF / `wemediaId` 属于 `direct` 通道**内部**的 preflight 细节（见 Decision 12），不改变执行位置，因此不配单独枚举。

**备选**：每个平台模块内部自己决定通道，注册表不暴露。放弃原因是无法校验「声明 webview 的平台必须存在页面脚本」「声明 direct 的平台不得依赖页面注入」，通道错误会在真机才暴露。当前 `auditRegistry()` 只做 `available` ↔ `ADAPTER_IMPLEMENTED_IDS` 的双向一致性校验（见 tasks 6.2），通道正确性由 `dispatch` 分支与真机验证保证——通道错配的代价是「发布直接失败」，不会静默污染数据，因此不额外加校验分支。

### Decision 2: 需要运行时签名的一律走 Webview，不逆向

凡契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway `keyv`+`ivd` 加密串 / 依赖页面脚本执行的平台，一律判 `webview` 通道。

**理由**：项目红线已明确「有签名 → 默认降级，不进实现阶段」。CSDN 是唯一例外（已授权、密钥已知、单层 HMAC）。知乎的双层签名（其中一层疑似 WASM 产出）与小红书、思否的页面加密串均不可控，逆向等于长期维护易变密钥。

**与「降级」的区别**：Webview 通道不是「降级为不支持」，而是**换执行位置**——算法仍在平台自己的页面里运行，本应用只搬运 payload。因此这 6 个平台在 v1.0.0 是**可交付**的，注册表已置 `available: true`。

**红线守卫**：页面脚本只允许调用平台页面已有的全局函数或已注册的模块方法，不得内联任何签名常量或算法实现。评审时以「脚本中是否出现签名函数名/密钥常量」为检查项（tasks 5.7）。

### Decision 3: Webview 通道用独立窗口 + 固定 label，与登录窗口分离

通道窗口 label 固定为 `channel-{platform}`，与登录窗口 `login-{platform}` 分离；窗口加载平台编辑页（如 `zhuanlan.zhihu.com/write`），注入 payload 后执行脚本，结果经 IPC 回传后关闭。

**理由**：登录窗口的 capability 是最小权限集且会在登录成功后关闭；通道窗口需要长期复用（同一平台连续同步多篇不必每次重新加载），也需要远程页面回传 IPC 的权限。混用同一个窗口会让「登录成功后自动关闭」与「通道窗口需常驻」互相冲突。

**补充**：`loginWindow: true` 的 `webview` 平台（知乎、简书、思否、小红书、头条、抖音）的「打开平台页面」按钮会拉起通道窗口本身，登录成功由 `watch_login` 检测 `is_logged_in_url(&url)` 后 `emit_success` 并立即关窗；后台批量同步走 `run()` 路径，不经过 `watch_login`，因此关窗不会打断同步。

### Decision 4: 通道回传单一结构化结果，与 `PublishResult` 一一映射

页面脚本回传 `{ platform, phase, message, isPublished?, postId?, editUrl?, stage? }`，Rust 侧映射为 `PublishResult`。`phase` 取 `success` / `failed` / `auth`，`draft` / `publish` 语义由 `stage = submit` + 保留 `postId` + `isPublished = false` 表达。

**理由**：两段式平台（知乎、抖音）的「建稿成功、发布失败」必须在结果里保留草稿 ID，否则会丢稿重稿。这与 phase2 Decision 5 确立的「部分成功仍写映射」语义完全一致，不需要新增字段。

### Decision 5: `direct` 通道的页面 token / CSRF / 身份预检统一进共享模块

`shared/page_token.rs` 提供页面 token 抽取，`shared/csrf.rs` 提供 CSRF 探针，二者都是 `direct` 通道**内部**的 preflight 能力（见 Decision 12），不是独立通道。三类抽取：页面 HTML 正则（公众号 `token`、微博 `config` JSON）、响应头读取（`x-ware-csrf-token`，探针须带 `x-secsdk-csrf-request: 1`，否则 404；该值作为 `x-secsdk-csrf-token` 发往业务请求）、Cookie 内直取（`bili_jct`）。抽取失败一律归类 `not_configured` 且**不发起业务请求**。

**理由**：各平台各自实现一遍正则与探针会产生多份漂移实现，而 token 抽取失败的表现是「业务请求被拒」，与凭据问题难以区分。收敛到一处后，失败文案可以统一为「请重新登录/重新复制 Cookie」。

**探针实现约束**（`shared/csrf.rs` 的 `probe_csrf_token`）：

- **版本头由调用方传入，不写死常量**。头条 `x-secsdk-csrf-version: 1.2.22`，掘金 `1.2.10`，两者不同且会随平台升级变化，写进共享层等于把平台细节耦合进来。
- **返回值支持两种取值方式**：整串（头条直接使用）与逗号第 N 段（掘金 `split(",")[1]`）。取值方式作为入参，不按平台名分支。

### Decision 6: 掘金维持草稿模式，不补直发分支

掘金 `publish_status = true` 时返回 `not_implemented`，并在结果文案中提示到掘金后台手动发布。原「补直发分支（`content_api/v1/article/publish`）」的结论**作废**。

**理由**：真机验证显示草稿路径稳定，但草稿转发布需要 `category_id` 与 `tag_ids` 的完整业务约束，且掘金后台的发布确认页有额外校验；在「绝不静默降级」的前提下，把能力如实标注为草稿模式、勾选发布时明确拒绝并提示手动发布，比引入一条未验证的直发分支更安全。注册表也不引入 `publishCapability` 字段（见 Decision 16），能力由适配器 `check_operation` 的实际行为表达。

### Decision 7: 双清单按通道重划，webview 平台不进 Cookie 白名单

- `LOGIN_SUPPORTED_PLATFORMS`：只保留 `direct` 且确有 Cookie 登录价值的平台（10 个）。
- `COOKIE_CREDENTIAL_PLATFORMS`：只保留 `credentialKind: "cookie"` 的平台（11 个）；`webview` 通道平台凭据不落 DPAPI Cookie，**不得列入**（否则导出时把 Webview 数据当 Cookie 解密会崩）。
- 博客园（`metaweblog`）继续排除。

**理由**：这是 project_rules 已写明的双清单硬约束。通道重划后，`webview` 平台的登录态由 Webview 数据目录持有，若仍列入 Cookie 白名单，导出/导入会拿到空凭据并误报「已配置」。

### Decision 8: 平台 id 与参考实现 bundle 对齐

注册表的 `tencent` 改名为 `tencentcloud`，与 `index.mjs` 的 `meta.id` 一致；其余 id 直接沿用 bundle 命名。

**理由**：id 是同步映射、日志、注册表、双清单的公共主键。若沿用自定义命名，每个平台都要维护一张「神兵 id ↔ 本地 id」映射表，任何一处漏改都会导致映射写错平台。改名成本集中在注册表与 `dispatch`，一次付清。

### Decision 9: 归集 phase2 / phase3，不再保留并行变更

phase2 与 phase3 的目录被删除，内容按性质归入本变更：

- **已交付且 spec 已同步进主 spec 的部分**（phase2 的 `platform-adapters` / `platform-shared-utils` / `sync-mapping` / `sync-logging` delta）：不重复搬运，主 spec 已是事实来源。
- **已交付但 spec 未同步的部分**（phase3 的 `app-settings` / `support-about` / `update-check` / `auth-demo` / `tauri-shell` delta）：迁入本变更 `specs/`，随本变更一起归档。
- **phase2 未完成的 4 项**（CSDN 守卫复活、CSDN 草稿更新、注册表能力位、集成验证）：并入本变更任务清单，编号改写为第 7 节，已完成。
- **phase2 的知乎 Cookie 直发设计**（Decision 4 两段式、Decision 11 降级结论）：作废，由 Decision 2 / Decision 3 的 Webview 通道取代。

**理由**：三份并行变更会让「v1.0.0 还差什么」需要跨三个目录拼装，且 phase2 的知乎任务与 phase4 的知乎方案互相矛盾。归集后单一目录即完整交付视图。

### Decision 10: CSDN 契约与已知风险（归集自 phase2 Decision 13，继续有效）

- 端点 `v3/mdeditor/saveArticle`，`status = 2` 草稿、`status = 0` 直发；请求参数用数字、列表接口返回字符串（`draft` / `audit` / `all_v2`），两套取值不可混用。
- 发布为**直接上线**（真机已验证：发布后进入「已发布」列表、审核计数为 0），因此 `is_published` 取 `request.publish_status`。
- 发布必须带标签（真机返回 `{"code":400,"msg":"请设置文章标签"}`）；草稿允许空标签。
- 唯一保留的拒绝：`existing_published = true` 且 `publish_status = false`（未勾选覆盖）时，在发请求前拒绝。
- 签名常量 `X_CA_KEY = 203803574`、`SECRET_KEY` 硬编码于 `csdn.rs`；CSDN 轮换密钥时会静默失效（`401` / `X-Ca-Key is not exist`），恢复方式是重取密钥只改该常量。这是已接受的固有维护成本（tasks 7.3 已演练）。
- 字段规范以 v3 为准：`readType`（驼峰）、`source: "pc_mdeditor"`、`pubStatus`；v1 的 `read_type` / `article_id` / `pc_postedit` 不可套用。

### Decision 11: 微信公众号维持「纯文字草稿 + 手动 Cookie/token」

不加入一键登录白名单，`requiredCookieKeys` 保持空数组，`requiredConfigKeys: ["token"]` 单独校验；`publish_status = true` 与 `existing_published = true` 返回 `not_implemented`（已发布文章不可改正文）。

**理由**：token 在后台 URL 里而不在 Cookie 里，扫码登录拿不到可用凭据；且窗口刚打开时微信就会种下游客 Cookie，空数组会让「刚开就已登录」的误判直接触发自动关闭。

### Decision 12: 页面 token / CSRF 预检不是独立通道，而是 `direct` 通道内的 preflight 能力

`channelType` 枚举固定为 `direct` / `webview` 两种。原 `http_page_token` 的四个平台（掘金、微信公众号、微博、头条）中，掘金 / 微信公众号 / 微博归入 `direct` 通道，其「先取 token 再发业务请求」的差异由 `shared/page_token.rs` 与 `shared/csrf.rs` 提供，作为适配器内部的 preflight 步骤；头条因直发被风控拒绝，改判 `webview`（Decision 17）。

**理由**：

1. **通道描述的是执行位置，不是请求次数**。`direct` 通道的语义是「Rust 直发、凭据为 Cookie + 静态头」，preflight 仍然是 Rust 直发，执行位置没变。把它升为通道会让同一个执行位置出现两个枚举值，`dispatch` 必须写两套几乎相同的分支。
2. **枚举每多一个值，就要在四处同步**：`registry.ts` 的类型与校验、前端按通道渲染凭据入口、`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`、`settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS`。四处分支只换来「多一次 GET/HEAD」，治理收益为零，返工面却翻倍。
3. **preflight 是平台级差异，不是通道级差异**。同属 `direct` 的阿里云需要 CSRF 头、哔哩哔哩从 Cookie 里直取 `bili_jct`、网易号需要 `wemediaId`、其余平台什么都不需要——这些差异本来就由各适配器自己处理，页面 token 没有理由例外。
4. **失败语义已经统一**。preflight 失败与凭据缺失都归类 `not_configured`、都提示「请重新登录」，行为上不需要区分通道。

**落地约束**：

- 注册表只出现两种 `channelType`；需要预检的平台在适配器内部调用共享层，不在注册表里额外打标。
- 若将来某平台的 preflight 需要多步跳转（如先访问首页拿会话 Cookie 再探针），仍归 `direct`，只增加适配器内部的步骤，不改枚举。

### Decision 13: 网易号与哔哩哔哩以草稿模式交付，`available: true`（原「保持 false」结论作废）

原判定把两者剔除在 v1.0.0 之外，理由是需要页面脚本参与执行。实际落地后二者都**以草稿模式交付并通过真机验证**，注册表 `available: true`，但都不提供真实发布：

- **网易号 `netease`**：先取 `wemediaId` / `realUserId`（`navinfo` 预检），再 `saveDraft`，全程纯 Cookie + 表单 POST，草稿路径成立。**正式发布**仍依赖页面脚本 `window.neg.getToken()` 产出的 `ursToken` 与叠加的 `X-NR-Trace-929` AES 加密头，Rust 侧无法自洽复现，因此 `publish_status = true` 返回 `not_implemented`。
- **哔哩哔哩 `bilibili`**：草稿走 `x/article/creative/draft/addupdate` 可纯 Cookie 完成，`bili_jct` 从 Cookie 直取作 CSRF。**正式发布**依赖 `member.bilibili.com/platform/upload/text/new-edit` 编辑器页内点按钮，且正文图片需先走上传流程，Rust 侧没有「打开编辑器页并点按钮」的能力，因此 `publish_status = true` 返回 `not_implemented`。

**结论**：两者都进 `dispatch` 的实现分支与 `SUPPORTED_PLATFORM_IDS`，注册表 `available: true`；但**只交付草稿能力**，勾选发布时如实返回 `not_implemented` 并提示手动发布。它们不进 `LOGIN_SUPPORTED_PLATFORMS`（`requiredCookieKeys` 为空，窗口刚开就会被误判已登录），进 `COOKIE_CREDENTIAL_PLATFORMS`（`credentialKind: "cookie"`，导出按 Cookie 处理）。

**理由**：与 Decision 16 的「绝不静默降级」一致——能力边界要如实写进返回值，而不是靠「平台不可用」把草稿能力一起砍掉。若后续补齐正式发布，应改走 `webview` 通道，而不是继续在 `direct` 通道内硬凑。

### Decision 14: `manualCookieOnly` 保留为兜底开关，当前无平台使用（原「6 平台走手动 Cookie」结论作废）

`PlatformDefinition` 的 `manualCookieOnly?: boolean` 字段保留，语义收窄为「仅在登录窗口确实拿不到可用 Cookie 时才使用」；当前 18 个平台**均未设置**该标记。

**作废原因**：原结论假设「`requiredCookieKeys` 为空 ⇒ 只能手动粘贴 Cookie」，但实际登录窗口已改为「URL 跳转 / DOM 探针确认登录态后抓取全量 Cookie」，因此 `requiredCookieKeys` 为空的平台（阿里云、51CTO、开源中国、腾讯云、百家号、网易号）同样支持扫码登录。前端默认同时提供「扫码登录」主入口与「手动粘贴」折叠备用入口。

**落地约束**：

- 前端不按 `requiredCookieKeys` 是否为空来推断「只能手动」，只按 `manualCookieOnly` 显式标记。
- 前端手动弹窗按 `requiredConfigKeys` 决定是否渲染额外输入框：仅微信公众号需要 `token`，其余平台只填 Cookie。
- `webview` 通道平台的凭据入口是「打开平台页面」，与手动 Cookie 入口无关。

### Decision 15: 简书改判 `webview` 通道——风控按客户端 TLS/HTTP 指纹拦截

`jianshu` 原登记为 `direct` 通道，2026-11-09 真机复测后改判为 `webview`：`GET /settings/basic.json` 在凭据完全有效时仍返回 `403 {"error":[{"code":9,"message":"异常请求"}]}`。

**排查过程（逐层排除）**：

1. **Cookie 完整**：`_m7e_session_core`、`remember_user_token`、`sensorsdata2015jssdkcross` 等键均在 WebView2 Cookie 管理器与 DPAPI 凭据里；`remember_user_token` 能解出真实用户 id 与签名。
2. **头部无效用**：补齐 `Origin` / `Referer` / `X-Requested-With: XMLHttpRequest` / `Sec-Fetch-Site` / `Sec-Fetch-Mode` / `Sec-Fetch-Dest` / `Accept` 变体 / 纯 Chrome UA 后仍 403；PowerShell（.NET TLS）、Node（OpenSSL，HTTP/1.1 与 HTTP/2）、Rust `reqwest` 三种客户端、五组头部变体结果一致。
3. **判定维度锁定指纹**：**不带 Cookie 的公开首页 `https://www.jianshu.com/` 同样 403**，而静态壳 `/writer` 返回 200。说明拦截发生在 TLS/HTTP 客户端指纹层（JA3 一类），与会话与头部组合无关。
4. **参考实现为何能过**：`docs/index.mjs` 的 `Px` 类跑在 Chrome 扩展后台 `runtime.fetch`，用的是 Chromium 自己的 TLS 栈，这一层从 Rust 侧无法复制。

**结论与落地**：

- `registry.ts`：`jianshu` 改 `credentialKind: "webview"`、`channelType: "webview"`、`loginWindow: true`；登录判据是 URL 从 `/sign_in` 跳回站内（`is_logged_in_url`），不依赖 Cookie 键。
- 适配器迁到 `platforms/webview/jianshu.rs`：登录与发布共用 `channel-jianshu` 窗口，四个接口（`/settings/basic.json`、`/author/notebooks`、`/author/notes`、`/author/notes/{id}/publicize`）全部在 `/writer` 页面上下文里同源 `fetch` 发出，TLS 指纹与 Cookie 由 WebView2 自己产生。
- 双清单：`jianshu` 移出 `LOGIN_SUPPORTED_PLATFORMS` 与 `COOKIE_CREDENTIAL_PLATFORMS`（按 project_rules 的 webview 通道约束）。
- 原 `platforms/jianshu.rs` 删除；`manualCookieOnly` 不再适用，手动粘贴 Cookie 入口随之关闭。
- 结果位：页面内已提交发布，但 `is_published` 恒为 `false`。**已确认为有意设计**（tasks 10.1）：简书按「仅草稿」口径登记，点发布后仍需平台审核、可见性无法即时确认，未取得可信证据前不置 `true`，属草稿模式的安全兜底。

**红线**：页面脚本只做「调接口、读结果」，不内联任何签名算法或密钥常量。

### Decision 16: 能力边界用返回值表达，注册表不引入 `publishCapability`

8 个仅草稿平台（掘金、微信公众号、阿里云、百家号、网易号、哔哩哔哩、抖音、简书）在 `publish_status = true` 时返回 `not_implemented`，结果文案明确提示「该平台暂不支持直接发布，请在平台后台手动发布」。`is_published` 如实反映真实状态（草稿即 `false`）。

**理由**：

1. **注册表字段是声明，适配器行为是事实**。两者一旦分离就会漂移：新增平台时容易只改注册表不改适配器，前端按声明渲染出「可发布」而实际返回 `not_implemented`。让能力只由 `check_operation` 的实际行为表达，就没有第二份事实。
2. **「绝不静默降级」的落点在返回值，不在 UI 提示**。勾选发布必须真发布；做不到就返回 `NotImplemented`，由前端按失败分类展示，而不是悄悄存草稿再报成功。
3. **草稿模式是交付内容，不是缺陷**。8 个平台的草稿路径真实可用（建稿、去重更新、写回映射），把它们标成不可用会把可用能力一起砍掉（见 Decision 13）。

**落地约束**：

- 适配器内 `publish_status` 与 `existing_published` 的组合必须显式判定，不允许「忽略参数、一律存草稿」。
- 前端按适配器返回的消息展示，不按平台 id 硬编码能力文案（当前仅掘金、抖音有硬编码草稿提示，属已知遗留，见 tasks 10.2）。

### Decision 17: 头条改判 `webview` 通道——直发被页面级风控拒绝

`toutiao` 原登记为 `direct` 通道（带 `x-ware-csrf-token` 预检）。真机验证中，即使把原始参考实现 `buildArticleBody` 的 27 个字段拼得一模一样、CSRF 探针正常返回，`reqwest` 直发仍稳定返回 `7050 参数校验失败`。

**根因**：参考实现的 `publishViaEditor` 是把 `fetch` **注入到头条编辑页的页面上下文**里执行，浏览器环境特征与页面 SDK 自动带上的字段才不会被判伪造。这属于「依赖页面脚本执行」的判定口径，与 Decision 2 的红线一致。

**结论与落地**：

- `registry.ts`：`toutiao` 改 `credentialKind: "webview"`、`channelType: "webview"`、`loginWindow: true`；登录判据是 URL 落到 `/profile_v4` 编辑页。
- 适配器迁到 `platforms/webview/toutiao.rs`，在 `channel-toutiao` 窗口的编辑页上下文内 `fetch`。
- 双清单：`toutiao` 移出 `LOGIN_SUPPORTED_PLATFORMS` 与 `COOKIE_CREDENTIAL_PLATFORMS`。
- `shared/csrf.rs` 的头条探针能力**保留**：它是通用能力，不因头条改判而删除（当前无调用方，但属共享层可复用资产）。

**红线**：页面脚本只做「调接口、读结果」，不内联任何签名算法或密钥常量。

## Risks / Trade-offs

- **Webview 通道依赖平台页面结构** → 平台改版即失效。缓解：页面脚本集中在 `platforms/webview/` 六个文件，失效时只改脚本；真机验收按平台逐个记录页面版本与验证日期。
- **通道窗口加载远程页面带来安全面** → 只给通道窗口声明最小 capability + 精确 `remote.urls`（只列实际需要的平台域名），不授予 store / opener / clipboard 权限。
- **`direct` 通道的预检是正则/探针，平台改版即失效** → 失败统一归类 `not_configured` 并提示重新登录，不做静默重试。
- **18 平台一次性铺开，单个平台契约错误会污染全局验收** → 任务按「契约对齐 → 适配器 → 单测 → dispatch → 真机」逐平台闭环，`available` 最后一步才置真；任一平台未通过则保持 `false`，不阻塞其余平台。
- **`tencent` → `tencentcloud` 改名** → 已同步注册表、`dispatch`、`SUPPORTED_PLATFORM_IDS` 与双清单。
- **归集删除 phase2/phase3 目录** → 已交付的 spec delta 若未同步进主 spec 会丢失。缓解：删除前逐项核对主 spec（`platform-adapters` / `platform-shared-utils` / `sync-mapping` / `sync-logging` 已确认同步），未同步的 phase3 delta 已迁入本变更。
- **8 个平台仅草稿** → 用户勾选发布时会看到失败提示。缓解：结果文案明确写「请在平台后台手动发布」，且 `is_published` 如实为 `false`；这是能力边界，不是 bug。
- **思否无测试账号** → 适配器已实现、单测通过、注册表已置 `available: true`，但真机未验证。缓解：tasks 8.7 保留为显式待验项，验证前不宣称已支持。

## Migration Plan

- 数据：凭据键空间沿用 `{platform}_cookie`（DPAPI）；sync_map 结构不变（`{id, published, bodyHash}`）。`webview` 通道平台不写凭据，无迁移。
- 顺序：共享层（`page_token` + `csrf`）→ Webview 通道骨架（仅知乎跑通）→ `direct` 全量平台 → 剩余 5 个 Webview 平台 → 注册表置真与双清单 → 集成验收 → 文档同步。
- 回滚：任一平台回滚只需把注册表 `available` 置 `false` 并从 `ADAPTER_IMPLEMENTED_IDS` 移除，适配器代码可保留（`dispatch` 未触发即不执行）。

## Open Questions

- ~~抖音注入脚本是否需要登录态之外的前置页面~~ 已关闭：`channel-douyin` 直接加载 `creator.douyin.com` 创作页，未登录会跳登录页，无需额外授权页。
- ~~小红书图文与长文是否共用签名助手~~ 已关闭：v1.0.0 只交付图文，长文不在范围，复用比例不再是阻塞项。
- ~~头条的 `x-secsdk-csrf-token` 探针是否需要 `ttwid` Cookie 预置~~ 已关闭（2026-11-08 真机验证，未登录态）：不需要 `ttwid` 预置，也不需要登录 Cookie。HEAD `https://mp.toutiao.com/ttwid/check/` 只需带 `x-secsdk-csrf-request: 1`（参考实现另带 `x-secsdk-csrf-version: 1.2.22`），响应头读 `x-ware-csrf-token`。头条后续整体改判 `webview`（Decision 17），探针能力保留在共享层。
- ~~网易号 / 哔哩哔哩是否在 v1.0.0 范围~~ 已关闭：均以草稿模式交付，`available: true`（Decision 13）。
- ~~**简书 `is_published` 恒为 `false`** 是否为有意为之~~ 已关闭：确认为有意设计，简书按「仅草稿」口径登记，内容验证与发布可见性未取得可信证据前不置 `true`，属安全兜底（tasks 10.1）。
- **思否真机验证**：缺测试账号，验证前 `available: true` 属「实现完成但未验证」，需在拿到账号后补验（tasks 8.7）。