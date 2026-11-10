# Design

## Context

本变更是 v1.0.0 的唯一总流程，归集 phase2（CSDN / 知乎）与 phase3（产品体验）。以下现状事实决定了方案选择：

- `platforms/mod.rs` 已定义 `PublishRequest`（`title` / `content` / `existing_id` / `is_update` / `publish_status` / `existing_published` / `tags`）与 `PublishResult`（`post_id` / `edit_url` / `message` / `failure_class` / `operation` / `is_published`），`dispatch` 用 `match platform` 路由。接入新平台不需要改请求/结果结构。
- 已交付 4 个适配器：`cnblogs`（MetaWeblog）、`juejin`（草稿 + 更新）、`csdn`（草稿 + 直发 + 阿里云网关 HMAC 签名）、`wechat`（纯文字草稿）。
- `packages/core/src/platforms/registry.ts` 只登记 9 个平台，`available` 与 `ADAPTER_IMPLEMENTED_IDS` 人工维护，`auditRegistry()` 做双向一致性校验。
- `login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`（当前 `["juejin"]`）与 `settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS` 是新平台接入的双清单，漏改会导致「能存 Cookie 却发不出去」或导出时解密崩溃。
- 现有 Webview 能力只用于**登录取 Cookie**（`login-*` 窗口 + `capabilities/login.json` 最小权限），没有任何「在平台页面里执行脚本」的通道。
- `docs/index.mjs` / `docs/index2.mjs` 是 19 个平台的完整参考实现（浏览器扩展形态），是本变更的契约事实来源。

## Goals / Non-Goals

**Goals:**

- 19 个国内平台全部出现在注册表，且每个平台都有明确通道、明确契约与明确验收标准
- `dispatch` 的路由逻辑、`PublishResult` 结构、`platform_publish` 命令签名零改动
- 需要运行时签名的平台**不逆向**，改为在其页面内执行其自身 JS
- 单个平台失败不影响其余平台；任何平台都不得把「发布」静默降级为「草稿」

**Non-Goals:**

- 不做海外平台（Dev.to / Hashnode / Medium），不在 v1.0.0 范围
- 不引入 Markdown 渲染库（沿用共享的「转义 + 每行 `<p>`」实现）
- 不做多账号矩阵、不做定时发布、不做审核状态轮询
- 不移植任何平台签名算法（唯一例外是已授权的 CSDN，已在基线中交付）

## 平台矩阵（契约事实来源：`docs/index.mjs` / `docs/index2.mjs`）

`index2.mjs` 覆盖 18 个平台，缺 `netease`；其余平台实现与 `index.mjs` 对齐，差异不构成阻塞。

| # | id | 名称 | 通道 | 主端点（神兵证据） | 鉴权 / 风险 | 批次 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `cnblogs` | 博客园 | `metaweblog` | MetaWeblog XML-RPC | Token，无签名 | 已交付 |
| 2 | `juejin` | 掘金 | `http` | `content_api/v1/article_draft/create`、`article_draft/update`、`article/publish`、`tag_api/v1/query_tag_list` | Cookie + `x-ware-csrf-token`（HEAD `user_api/v1/sys/token` 探针，须带 `x-secsdk-csrf-request: 1`，否则 404；取值作为 `x-secsdk-csrf-token` 发业务请求）；需页面 token 预检 | 已交付草稿，补直发 |
| 3 | `csdn` | CSDN | `http` | `bizapi.csdn.net/blog-console-api/v3/mdeditor/saveArticle` | Cookie + CSRF + 网关 HMAC（已授权例外） | 已交付 |
| 4 | `wechat` | 微信公众号 | `http` | `mp.weixin.qq.com/cgi-bin/operate_appmsg`、`filetransfer` | Cookie + URL `token`（页面 HTML 正则 `t:\s*["']…`）；需页面 token 预检 | 已交付草稿 |
| 5 | `jianshu` | 简书 | `webview` | `www.jianshu.com/author/notes`、`/publicize`、`/settings/basic.json` | Cookie 完整仍 403 `code 9`：风控按客户端 TLS/HTTP 指纹拦截（见 Decision 15） | 已交付（通道改判） |
| 6 | `51cto` | 51CTO | `http` | `blog.51cto.com/blogger/publish` + 存储上传 | Cookie，无签名 | A |
| 7 | `oschina` | 开源中国 | `http` | `apiv1.oschina.net` 草稿/发布 | Cookie，无签名 | A |
| 8 | `tencentcloud` | 腾讯云开发者社区 | `http` | `cloud.tencent.com/developer/api/*` | Cookie，无签名 | A |
| 9 | `aliyun` | 阿里云开发者社区 | `http` | `developer.aliyun.com/developer/api/article/publishArticle` | Cookie + `csrfHeaders` | A |
| 10 | `weibo` | 微博 | `http` | `card.weibo.com/article/v5/editor` | Cookie + 页面 `config` JSON 正则；需页面 token 预检 | A |
| 11 | `baijiahao` | 百家号 | `http` | `baijiahao.baidu.com/pcui/article/save?callback=bjhdraft`、`materialui/picture/uploadProxy` | Cookie，无签名 | A |
| 12 | `netease` | 网易号 | `http` | `mp.163.com` 表单 POST | Cookie，无签名 | A |
| 13 | `toutiao` | 头条 | `http` | `mp.toutiao.com/mp/agw/*`、`ttwid/check/` | Cookie + `x-ware-csrf-token`（HEAD `ttwid/check/` 探针，须带 `x-secsdk-csrf-request: 1`，否则 404；取值作为 `x-secsdk-csrf-token` 发业务请求）；需页面 token 预检 | A |
| 14 | `bilibili` | 哔哩哔哩 | `http` | `api.bilibili.com`、`member.bilibili.com/platform/upload/text` | Cookie + `bili_jct`（Cookie 内自带 CSRF） | A |
| 15 | `zhihu` | 知乎 | `webview` | `zhuanlan.zhihu.com/api/articles/drafts`、`api/v4/me`、`api.zhihu.com/images` | `x-zse-93` / `x-zse-96` / `x-zst-81` + `ZP` 函数，**全站双层签名** | B |
| 16 | `xiaohongshu` | 小红书长文笔记 | `webview` | `creator.xiaohongshu.com`、`api/galaxy/user/info` | `_webmsxyw` / `X-s-common`（`edith.xiaohongshu.com`） | B |
| 17 | `xiaohongshu-image` | 小红书图文笔记 | `webview` | `creator.xiaohongshu.com` 注入 + 共享签名助手 | 同上 | B |
| 18 | `douyin` | 抖音长文 | `webview` | `creator.douyin.com/*` 注入脚本 | `x-tt-verify-passport-decision` 等运行时校验 | B |
| 19 | `douyin-image` | 抖音图文 | `webview` | `creator.douyin.com` + `/web_api/sns/v2/note` | 同上 | B |
| 20 | `segmentfault` | 思否 | `webview` | `segmentfault.com/gateway/{path}?keyv=&ivd=` | 页面内 `encrypt(JSON.stringify(...))` 生成加密串 | B |

> 表中 20 行含 CSDN（神兵 bundle 外由本项目实现），平台 id 集合仍为 19 个；`index2.mjs` 不含 `netease`，网易号契约以 `index.mjs` 为准。

通道计数：`http` 12 个（其中 `juejin` / `wechat` / `weibo` / `toutiao` 需页面 token 预检）、`webview` 7 个、`metaweblog` 1 个。

## Decisions

### Decision 1: 通道是注册表的一等能力位，而不是实现细节

`PlatformDefinition` 新增 `adapterChannel: "http" | "webview" | "metaweblog"`，与既有 `credentialKind` **正交**：`credentialKind` 决定前端如何收集凭据，`adapterChannel` 决定后端如何执行请求。

**理由**：`credentialKind` 已经无法表达「凭据怎么收」与「请求怎么发」的差异。例如知乎是 `cookie` 凭据但必须 `webview` 执行；微博是 `cookie` 凭据但需要先 GET 一次页面抽 token。把它们混成一个枚举会迫使前端按平台 id 硬编码判断，正是 phase2 Decision 6 要消除的模式。

**为什么只有三种枚举**：`http` / `webview` / `metaweblog` 对应三种**执行位置**——Rust 直发、平台页面内执行、XML-RPC 直发。是否需要先取页面 token 属于 `http` 通道**内部**的 preflight 细节（见 Decision 12），不改变执行位置，因此不配单独枚举。若把 preflight 提为通道，`auditRegistry`、前端渲染与双清单都要为它多写一条分支，而它带来的差异仅是「多一次 GET/HEAD」，没有治理价值。

**备选**：每个平台模块内部自己决定通道，注册表不暴露。放弃原因是 `auditRegistry()` 无法校验「声明 webview 的平台必须存在页面脚本」「声明 http 的平台不得依赖页面注入」，通道错误会在真机才暴露。

### Decision 2: 需要运行时签名的一律走 Webview，不逆向

凡契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway `keyv`+`ivd` 加密串 / 依赖 `runtime.tabs.executeScript` 的平台，一律判 `webview` 通道。

**理由**：项目红线已明确「有签名 → 默认降级，不进实现阶段」。CSDN 是唯一例外（已授权、密钥已知、单层 HMAC）。知乎的双层签名（其中一层疑似 WASM 产出）与小红书、思否的页面加密串均不可控，逆向等于长期维护易变密钥。

**与「降级」的区别**：Webview 通道不是「降级为不支持」，而是**换执行位置**——算法仍在平台自己的页面里运行，本应用只搬运 payload。因此这 6 个平台在 v1.0.0 是**可交付**的，注册表最终置 `available: true`。

**红线守卫**：页面脚本只允许调用平台页面已有的全局函数或已注册的模块方法，不得内联任何签名常量或算法实现。评审时以「脚本中是否出现签名函数名/密钥常量」为检查项。

### Decision 3: Webview 通道用独立窗口 + 固定 label，与登录窗口分离

通道窗口 label 固定为 `channel-{platform}`，与登录窗口 `login-{platform}` 分离；窗口加载平台编辑页（如 `zhuanlan.zhihu.com/write`），注入 payload 后执行脚本，结果经 IPC 回传后关闭。

**理由**：登录窗口的 capability 是最小权限集且会在登录成功后关闭；通道窗口需要长期复用（同一平台连续同步多篇不必每次重新加载），也需要远程页面回传 IPC 的权限。混用同一个窗口会让「登录成功后自动关闭」与「通道窗口需常驻」互相冲突。

**备选**：复用登录窗口。放弃原因是登录窗口的 `on_page_load(Finished)` 回调绑定在「Cookie 齐全即成功关闭」语义上，通道窗口需要的是「页面脚本执行完毕才回传」，两套生命周期无法共存。

### Decision 4: 通道回传单一结构化结果，与 `PublishResult` 一一映射

页面脚本返回 `{ success, postId, editUrl, message, phase, isPublished }`，Rust 侧原样映射为 `PublishResult`。`phase` 取 `auth` / `draft` / `publish`，用于把失败归类到具体阶段。

**理由**：两段式平台（知乎、抖音）的「建稿成功、发布失败」必须在结果里保留草稿 ID，否则会丢稿重稿。这与 phase2 Decision 5 确立的「部分成功仍写映射」语义完全一致，不需要新增字段。

### Decision 5: `http` 通道的页面 token / CSRF 预检统一进共享模块

`shared/page_token.rs` 提供页面 token 抽取，`shared/csrf.rs` 提供 CSRF 探针，二者都是 `http` 通道**内部**的 preflight 能力（见 Decision 12），不是独立通道。三类抽取：页面 HTML 正则（公众号 `t`、微博 `config`）、响应头读取（`x-ware-csrf-token`，探针须带 `x-secsdk-csrf-request: 1`，否则 404；该值作为 `x-secsdk-csrf-token` 发往业务请求）、Cookie 内直取（`bili_jct`）。抽取失败一律归类 `not_configured` 且**不发起业务请求**。

**理由**：四个平台各自实现一遍正则与探针会产生四份漂移实现，而 token 抽取失败的表现是「业务请求被拒」，与凭据问题难以区分。收敛到一处后，失败文案可以统一为「请重新登录/重新复制 Cookie」。

**探针实现约束**（`shared/csrf.rs` 的 `probe_csrf_token`）：

- **版本头由调用方传入，不写死常量**。头条 `x-secsdk-csrf-version: 1.2.22`，掘金 `1.2.10`，两者不同且会随平台升级变化，写进共享层等于把平台细节耦合进来。
- **返回值支持两种取值方式**：整串（头条直接使用）与逗号第 N 段（掘金 `split(",")[1]`）。取值方式作为入参，不按平台名分支。

### Decision 6: 掘金补直发分支，复用既有草稿路由维度

`publish_status = true` 且目标未发布时走 `content_api/v1/article/publish`；`existing_published = true` 时按既有约束需要显式覆盖。掘金 `publishCapability` 由 `draft_only` 改为 `live`。

**理由**：神兵已给出该端点与请求体（`article_id` / `category_id` / `tag_ids`），无需重新抓包；掘金是首发主力平台，只支持草稿会让「平推」在第一个平台就出现能力缺口。

### Decision 7: 双清单按通道重划，webview 平台不进 Cookie 白名单

- `LOGIN_SUPPORTED_PLATFORMS`：只保留 `http` 且确有 Cookie 登录价值的平台。
- `COOKIE_CREDENTIAL_PLATFORMS`：只保留 `credentialKind: "cookie"` 且凭据是 Cookie 的平台；`webview` 通道平台凭据不落 DPAPI Cookie，**不得列入**（否则导出时把 Webview 数据当 Cookie 解密会崩）。
- 博客园（`metaweblog`）继续排除。

**理由**：这是 project_rules 已写明的双清单硬约束。通道重划后，`webview` 平台的登录态由 Webview 数据目录持有，若仍列入 Cookie 白名单，导出/导入会拿到空凭据并误报「已配置」。

### Decision 8: 平台 id 与神兵 bundle 对齐

注册表的 `tencent` 改名为 `tencentcloud`，与 `index.mjs` 的 `meta.id` 一致；其余 id 直接沿用 bundle 命名（含 `douyin-image` / `xiaohongshu-image` 这类带后缀的图文平台）。

**理由**：id 是同步映射、日志、注册表、双清单的公共主键。若沿用自定义命名，每个平台都要维护一张「神兵 id ↔ 本地 id」映射表，任何一处漏改都会导致映射写错平台。改名成本集中在注册表与 `dispatch`，一次付清。

**迁移**：`tencent` 尚未实现（`available: false`），改名不涉及已存数据。

### Decision 9: 归集 phase2 / phase3，不再保留并行变更

phase2 与 phase3 的目录被删除，内容按性质归入本变更：

- **已交付且 spec 已同步进主 spec 的部分**（phase2 的 `platform-adapters` / `platform-shared-utils` / `sync-mapping` / `sync-logging` delta）：不重复搬运，主 spec 已是事实来源。
- **已交付但 spec 未同步的部分**（phase3 的 `app-settings` / `support-about` / `update-check` / `auth-demo` / `tauri-shell` delta）：迁入本变更 `specs/`，随本变更一起归档。
- **phase2 未完成的 4 项**（CSDN 守卫复活、CSDN 草稿更新、注册表能力位、集成验证）：并入本变更任务清单，编号改写为第 7 节。
- **phase2 的知乎 Cookie 直发设计**（Decision 4 两段式、Decision 11 降级结论）：作废，由 Decision 2 / Decision 3 的 Webview 通道取代。

**理由**：三份并行变更会让「v1.0.0 还差什么」需要跨三个目录拼装，且 phase2 的知乎任务与 phase4 的知乎方案互相矛盾。归集后单一目录即完整交付视图。

### Decision 10: CSDN 契约与已知风险（归集自 phase2 Decision 13，继续有效）

- 端点 `v3/mdeditor/saveArticle`，`status = 2` 草稿、`status = 0` 直发；请求参数用数字、列表接口返回字符串（`draft` / `audit` / `all_v2`），两套取值不可混用。
- 发布为**直接上线**（真机已验证：发布后进入「已发布」列表、审核计数为 0），因此 `is_published` 取 `request.publish_status`。
- 发布必须带标签（真机返回 `{"code":400,"msg":"请设置文章标签"}`）；草稿允许空标签。
- 唯一保留的拒绝：`existing_published = true` 且 `publish_status = false`（未勾选覆盖）时，在发请求前拒绝。
- 签名常量 `X_CA_KEY = 203803574`、`SECRET_KEY` 硬编码于 `csdn.rs`；CSDN 轮换密钥时会静默失效（`401` / `X-Ca-Key is not exist`），恢复方式是重取密钥只改该常量。这是已接受的固有维护成本。
- 字段规范以 v3 为准：`readType`（驼峰）、`source: "pc_mdeditor"`、`pubStatus`；v1 的 `read_type` / `article_id` / `pc_postedit` 不可套用。

### Decision 11: 微信公众号维持「纯文字草稿 + 手动 Cookie/token」

不加入一键登录白名单，`requiredCookieKeys` 保持空数组，`requiredConfigKeys: ["token"]` 单独校验；`publish_status = true` 与 `existing_published = true` 返回 `not_implemented`（已发布文章不可改正文）。

**理由**：token 在后台 URL 里而不在 Cookie 里，扫码登录拿不到可用凭据；且窗口刚打开时微信就会种下游客 Cookie，空数组会让「刚开就已登录」的误判直接触发自动关闭。

### Decision 12: `http_page_token` 不是独立通道，而是 `http` 通道内的 preflight 能力

`adapterChannel` 枚举固定为 `http` / `webview` / `metaweblog` 三种。原 `http_page_token` 的四个平台（掘金、微信公众号、微博、头条）一律归入 `http` 通道，其「先取 token 再发业务请求」的差异由 `shared/page_token.rs` 与 `shared/csrf.rs` 提供，作为适配器内部的 preflight 步骤。

**理由**：

1. **通道描述的是执行位置，不是请求次数**。`http` 通道的语义是「Rust 直发、凭据为 Cookie + 静态头」，preflight 仍然是 Rust 直发，执行位置没变。把它升为通道会让同一个执行位置出现两个枚举值，`dispatch` 必须写两套几乎相同的分支。
2. **枚举每多一个值，就要在四处同步**：`registry.ts` 的类型与 `auditRegistry` 校验、前端按通道渲染凭据入口、`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`、`settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS`。四处分支只换来「多一次 GET/HEAD」，治理收益为零，返工面却翻倍。
3. **preflight 是平台级差异，不是通道级差异**。同属 `http` 的阿里云需要 CSRF 头、哔哩哔哩从 Cookie 里直取 `bili_jct`、其余平台什么都不需要——这些差异本来就由各适配器自己处理，页面 token 没有理由例外。
4. **失败语义已经统一**。preflight 失败与凭据缺失都归类 `not_configured`、都提示「请重新登录」，行为上不需要区分通道。

**落地约束**：

- 注册表只出现三种 `adapterChannel`；需要预检的平台在适配器内部调用共享层，不在注册表里额外打标。
- 若将来某平台的 preflight 需要多步跳转（如先访问首页拿会话 Cookie 再探针），仍归 `http`，只增加适配器内部的步骤，不改枚举。

### Decision 13: 网易号与哔哩哔哩不满足「纯 Cookie 直发」，保持 `available: false`

第 4 节按「纯 Cookie 直发」筛选后，网易号与哔哩哔哩被剔除，其余 6 个（简书、51CTO、开源中国、腾讯云、阿里云、百家号）已交付。

- **网易号 `netease`**：`wemedia/article/status/api/publishV2.do` 的表单需要 `ursToken`，而该 token 只能由**页面脚本** `window.neg.getToken()` 产出（bundle 里是 `executeScript` 调 `neg.getToken()`），不存在等价的 HTTP 获取路径；请求体还叠加了一层加密参数。凭 Cookie 无法在 Rust 侧自洽复现。
- **哔哩哔哩 `bilibili`**：草稿走 `x/article/creative/draft/addupdate` 可以纯 Cookie 完成，但**正式发布**依赖 `member.bilibili.com/platform/upload/text/new-edit` 编辑器页：bundle 用 `tabs.create` + 注入脚本 + 拦截 `XMLHttpRequest.prototype.send` 点击发布（`submitFromEditorPage`），并且正文图片必须先走上传流程。Rust 侧没有「打开编辑器页并点按钮」的能力。

**结论**：两者都不进 `dispatch` 的实现分支（`SUPPORTED_PLATFORM_IDS` 命中即返回 `NotImplemented`），注册表保持 `available: false`，登录白名单不含它们。`COOKIE_CREDENTIAL_PLATFORMS` 仍按注册表的 `credentialKind: "cookie"` 登记：虽然当前存不进凭据，但导出时按 Cookie 凭据处理才不会把 `platform_config.json` 里的明文 Cookie 当普通配置导出，两边清单也保持一致。后续若补齐，应改走通道 B（`webview`），而不是继续在 `http` 通道内硬凑。

**理由**：与 Decision 2 一致——需要页面脚本参与执行（取 token、点按钮、拦截 XHR）的路径，执行位置就必须是平台页面本身；在 Rust 侧逆向页面加密属于 project_rules 明令不做的范围。

### Decision 14: 新增 `manualCookieOnly` 标记，第 4 节 6 个平台走手动 Cookie 入口

`registry.ts` 的 `PlatformDefinition` 增加可选字段 `manualCookieOnly?: boolean`，简书、51CTO、开源中国、腾讯云、阿里云、百家号均置 `true`；前端凭据弹窗据此直接渲染 Cookie 输入框，不再展示「扫码登录」按钮。

**理由**：

1. **一键登录的完成判据是 `requiredCookieKeys` 非空**。这 6 个平台没有可靠的登录态 Cookie 键（阿里云的 `c_csrf` 是防 CSRF 令牌，未登录时也存在），若加入 `LOGIN_SUPPORTED_PLATFORMS`，窗口刚打开就会被判定「已登录」并自动关闭，存下一个匿名 Cookie。
2. **但 `available: true` 会解锁配置入口**，前端默认把 `requiredCookieKeys` 为空的 Cookie 平台渲染成「扫码登录」，用户点下去只会拿到 `persist_cookie` 的 allowlist 拒绝。用显式标记把两者解耦，比在 `handleOpenLogin` 里静默吞掉错误更清晰。
3. **标记只影响前端入口，不影响发布链路**：这 6 个平台的凭据仍写入 `platform_config.json` 的 `{id}_config.cookie`，由 `COOKIE_CREDENTIAL_PLATFORMS` 覆盖导出/导入，Rust 适配器读配置的路径不变。

**落地约束**：

- `manualCookieOnly: true` 的平台**不得**列入 `login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`（与 `webview` 通道同一约束：不能用一个注定拿不到可用凭据的登录窗口去抓 Cookie）。
- 前端手动弹窗按 `requiredConfigKeys` 决定是否渲染额外输入框：仅微信公众号需要 `token`，其余平台只填 Cookie。

### Decision 15: 简书改判 `webview` 通道——风控按客户端 TLS/HTTP 指纹拦截

`jianshu` 原登记为 `http` 通道 + `manualCookieOnly`，2026-11-09 真机复测后改判为 `webview`：`GET /settings/basic.json` 在凭据完全有效时仍返回 `403 {"error":[{"code":9,"message":"异常请求"}]}`。

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

**红线**：页面脚本只做「调接口、读结果」，不内联任何签名算法或密钥常量。

## Risks / Trade-offs

- **Webview 通道依赖平台页面结构** → 平台改版即失效。缓解：页面脚本集中在 `platforms/webview/` 六个文件，失效时只改脚本；真机验收按平台逐个记录页面版本与验证日期。
- **通道窗口加载远程页面带来安全面** → 只给通道窗口声明最小 capability + 精确 `remote.urls`（只列 6 个平台域名），不授予 store / opener / clipboard 权限。
- **`http` 通道的页面 token 预检是正则，平台改版即失效** → 失败统一归类 `not_configured` 并提示重新登录，不做静默重试。
- **19 平台一次性铺开，单个平台契约错误会污染全局验收** → 任务按「契约对齐 → 适配器 → 单测 → dispatch → 真机」逐平台闭环，`available` 最后一步才置真；任一平台未通过则保持 `false`，不阻塞其余平台。
- **`tencent` → `tencentcloud` 改名** → 只涉及未实现平台，无数据迁移；但需同时改注册表、`dispatch`、`SUPPORTED_PLATFORM_IDS`，漏一处会返回「未知平台」。
- **归集删除 phase2/phase3 目录** → 已交付的 spec delta 若未同步进主 spec 会丢失。缓解：删除前逐项核对主 spec（`platform-adapters` / `platform-shared-utils` / `sync-mapping` / `sync-logging` 已确认同步），未同步的 phase3 delta 已迁入本变更。

## Migration Plan

- 数据：凭据键空间沿用 `{platform}_cookie`（DPAPI）；sync_map 结构不变（`{id, published, bodyHash}`）。`webview` 通道平台不写凭据，无迁移。
- 顺序：共享层（`page_token` + `csrf`）→ Webview 通道骨架（仅知乎跑通）→ `http` 全量 13 平台 → 剩余 5 个 Webview 平台 → 注册表置真与双清单 → 集成验收。
- 回滚：任一平台回滚只需把注册表 `available` 置 `false` 并从 `ADAPTER_IMPLEMENTED_IDS` 移除，适配器代码可保留（`dispatch` 未触发即不执行）。

## Open Questions

- 抖音 / 抖音图文的注入脚本是否需要登录态之外的前置页面（如创作者中心授权页）——影响通道窗口的初始 URL，不影响通道设计。
- 小红书图文笔记是否与长文共用同一套签名助手——影响页面脚本可复用的比例，不影响通道选型。
- ~~头条的 `x-secsdk-csrf-token` 探针是否需要 `ttwid` Cookie 预置~~ 已关闭（2026-11-08 真机验证，未登录态）：不需要 `ttwid` 预置，也不需要登录 Cookie。HEAD `https://mp.toutiao.com/ttwid/check/` 只需带 `x-secsdk-csrf-request: 1`（参考实现另带 `x-secsdk-csrf-version: 1.2.22`），响应头读 `x-ware-csrf-token`，取值作为 `x-secsdk-csrf-token` 发往业务请求。缺请求头返回 404，归 `not_configured`。