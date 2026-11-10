# Tasks

> 契约事实来源：`docs/index.mjs`（19 平台）、`docs/index2.mjs`（18 平台，缺网易号）。每个平台按「契约对齐 → 适配器 → 单测 → dispatch 注册 → 真机验证」闭环，真机通过后才把 `available` 置 `true`。

**通道口径**：`adapterChannel` 只有 `http` / `webview` / `metaweblog` 三种（见 `design.md` Decision 12）。需要先取页面 token 或响应头 CSRF 的平台仍归 `http`，其预检由 `shared/page_token.rs` 与 `shared/csrf.rs` 提供。

## 推荐执行顺序

`0 基线 → 2 共享层 → 3 Webview 通道骨架（仅知乎跑通）→ 4 http 全量平台（原 10 项 + 并入的掘金 / 微信公众号共 12 项）→ 5 剩余 5 个 Webview 平台 → 6 注册表收口 → 7 归集遗留 → 8 集成验收`

理由：共享层是 `http` 通道里 4 个预检平台的共同底座，先落地才不会四份正则漂移；Webview 通道先用知乎打通端到端，其余 5 个平台只是换页面脚本，不重复验证通道骨架；注册表置真必须放在所有适配器之后，避免 `available` 先于能力存在。

## 0. 已交付基线（归集自 phase1 / phase2 / phase3）

- [x] 0.1 phase1：博客园 MetaWeblog 适配器、掘金草稿适配器、同步映射、同步日志、注册表单一来源
- [x] 0.2 phase2：共享工具上提（cookie / csrf / markdown / truncate / extract）、两段式部分成功映射、CSDN 草稿 + 直发 + 网关 HMAC 签名、微信公众号纯文字草稿
- [x] 0.3 phase3：关于/支持弹窗、一键登录、设置中心与配置导出/导入/重置、版本检测、启动白屏与首屏提示（42 项）

## 1. 契约固化（神兵对齐）

- [ ] 1.1 把 `design.md` 的 19 平台矩阵逐行核对到 `index.mjs`：端点、字段名、请求头、CSRF 来源、通道判定依据
- [ ] 1.2 交叉核对 `index2.mjs`，确认差异只影响网易号（`index2.mjs` 缺该平台）
- [ ] 1.3 每个平台标注 `publishCapability`（`live` / `draft_only` / `unsupported`）与失败类别映射
- [ ] 1.4 标注每个 `http` 平台的预检类型：`none` / `page_html` / `response_header` / `cookie`，作为共享层调用方的契约

## 2. 共享层（`platforms/shared/`）

- [x] 2.1 `page_token.rs`：从页面 HTML 抽取 token（公众号 `token`、微博 `config` JSON 字段）
- [x] 2.2 `csrf.rs` 扩展：`probe_csrf_token` 走 HEAD 请求读响应头 `x-ware-csrf-token`（头条 URL `https://mp.toutiao.com/ttwid/check/`、掘金 URL `https://api.juejin.cn/user_api/v1/sys/token`；均须带 `x-secsdk-csrf-request: 1`，缺请求头返回 404）。两条约束：`x-secsdk-csrf-version` 由调用方传入（头条 `1.2.22`、掘金 `1.2.10`），不写死常量；返回值支持整串与逗号第 N 段两种取值方式（头条用整串、掘金用 `split(",")[1]`）
- [x] 2.3 单测：token 命中、缺失、非法 JSON 三类，失败均返回错误而非空串
- [x] 2.4 单测：探针超时与响应头缺失均归类 `not_configured`，且不自动重试
- [x] 2.5 确认 token 不写入凭据存储与日志

## 3. Webview 注入通道（`platforms/webview/`）

- [x] 3.1 通道窗口管理：label `channel-{platform}`、复用、聚焦、关闭后重建
- [x] 3.2 页面就绪判定与 payload 注入（只含文章数据字段）
- [x] 3.3 IPC 回传协议：`{ platform, phase, message, isPublished?, postId?, editUrl?, stage? }` → `PublishResult`；`phase` 取 `success` / `failed` / `auth`，`draft` / `publish` 语义由 `stage=submit` + 保留 `postId` + `isPublished=false` 表达
- [x] 3.4 四段超时（加载 / 就绪 / 执行 / 回传）与失败分类，全部归类 `network_error`
- [x] 3.5 `capabilities/webview-channel.json`：6 个平台域名精确列入 `remote.urls`，不含 store / clipboard / opener
- [x] 3.6 单测：占位 ID 不落盘、`auth` 阶段终止不发请求、回传超时不算成功
- [ ] 3.7 先用知乎跑通通道端到端（骨架验证样本），再复用到其余 5 个平台

## 4. 通道 A：`http` 直发（12 项待办 + CSDN 已交付）

- [x] 4.1 简书 `jianshu`：建笔记 → 发布 → 返回真实标识与编辑地址（`publicize` 缺 `last_compiled_at` 视为发布失败）
- [x] 4.2 51CTO `51cto`：提交标题/正文/标签，标识写入映射（CSRF 取不到直接归 `auth_error`）
- [x] 4.3 开源中国 `oschina`：创建或更新，失败归类明确（`user/myDetails` 拿不到 userId 归 `auth_error`）
- [x] 4.4 腾讯云开发者社区 `tencentcloud`：注册表 id 由 `tencent` 改名，`dispatch` / 双清单 / 配置键同步（正文用 `<!--markdown-->` 包裹）
- [x] 4.5 阿里云开发者社区 `aliyun`：带 CSRF 头提交，取不到 CSRF 时不发业务请求（正式发布依赖编辑器页面，当前仅草稿）
- [x] 4.6 百家号 `baijiahao`：保存/提交内容，标识写入映射（JSONP 解析；正式发布依赖页内按钮与可能的人机验证，当前仅草稿）
- [x] 4.7 网易号 `netease`（`http` + 预检 `navinfo`）：先取 `wemediaId`/`realUserId` 再 `saveDraft`；正式发布依赖页面 `neg.getToken()` + `X-NR-Trace-929` AES 加密头，`publish_status = true` 返回 `not_implemented`
- [x] 4.8 哔哩哔哩 `bilibili`（`http` + 预检 `nav`）：从 Cookie 读 `bili_jct` 作 CSRF，缺失时归类 `not_configured`；草稿走 `draft/addupdate`，正式发布依赖编辑页内按钮，`publish_status = true` 返回 `not_implemented`
- [x] 4.9 微博 `weibo`（`http` + 预检 `page_html`）：先抽编辑器配置 JSON 再提交
- [x] 4.10 头条 `toutiao`（`http` + 预检 `response_header`）：先探针取 CSRF 再提交
- [ ] 4.11 掘金 `juejin`（`http` + 预检 `response_header`）：补直发分支 `article/publish`，`publishCapability` 改为 `live`
- [ ] 4.12 微信公众号 `wechat`（`http` + 预检 `page_html`）：维持纯文字草稿，`publish_status = true` 与已发布覆盖返回 `not_implemented`
- [ ] 4.13 单测：每个平台覆盖成功、鉴权失败、预检失败、平台错误码四类
- [ ] 4.14 单测：草稿成功但发布失败时 `is_published` 为 `false` 且保留草稿标识
- [ ] 4.15 单测：预检失败时业务请求零发起（用请求计数断言，而非只看返回码）

> 4.9–4.12 即原 `http_page_token` 通道的四个平台，现归入 `http` 通道，差异只体现在「发业务请求前多一次预检」，`dispatch` 分支与其余 `http` 平台共用。

## 5. 通道 B：Webview 注入（6 个平台）

> 第 5 节的知乎是 Webview 通道的验证样本，必须在第 4 节任一平台跑通之后再开始其余 5 个 Webview 平台。

- [ ] 5.1 知乎 `zhihu`：编辑页建稿 + 勾选发布，返回真实标识；替代 phase2 的 Cookie 直发方案
- [ ] 5.2 小红书长文笔记 `xiaohongshu`：创作页提交，不计算运行时签名头
- [ ] 5.3 小红书图文笔记 `xiaohongshu-image`：图文一并提交
- [ ] 5.4 抖音长文 `douyin`：创作页提交，标识写入映射
- [ ] 5.5 抖音图文 `douyin-image`：图文提交，标识写入映射
- [ ] 5.6 思否 `segmentfault`：页面内调用其网关，不移植加密参数生成
- [ ] 5.7 页面脚本红线审查：6 份脚本均无签名函数名、密钥常量、加密串拼接
- [ ] 5.8 单测：页面内未登录在 `auth` 阶段终止；单平台失败不影响其他平台

## 6. 注册表与双清单

- [ ] 6.1 `registry.ts` 补齐 19 条，新增 `adapterChannel` 与 `publishCapability`，`tencent` → `tencentcloud`
- [ ] 6.2 `auditRegistry()` 增加通道校验：`webview` 平台必须有页面脚本，`http` 平台不得依赖页面注入
- [ ] 6.3 `available` 与 `ADAPTER_IMPLEMENTED_IDS` 逐平台对齐，未真机通过的一律 `false`
- [ ] 6.4 `login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS` 按通道重划（排除 6 个 `webview` 平台与博客园）
- [ ] 6.5 `settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS` 按通道重划（排除 `webview` 平台与 `metaweblog`）
- [ ] 6.6 `mod.rs` 的 `SUPPORTED_PLATFORM_IDS` 与 `dispatch` 分支同步到 19 个
- [ ] 6.7 前端按 `adapterChannel` 渲染凭据入口：`webview` 平台展示「打开平台页面」而非「一键登录」
- [ ] 6.8 构建顺序验证：`packages/core` 执行 `pnpm build` → `apps/desktop-app` 构建通过

## 7. 归集遗留项（原 phase2 未完成）

- [ ] 7.1 CSDN 守卫复活后的直发真机验证（原 4.10d：新建 → 草稿同步 → 勾选发布 → 不勾选覆盖再同步，预期被拦截且不发请求）
- [ ] 7.2 CSDN 草稿更新分支真机验证（原 4.12：改内容再同步不产生重复文章）
- [ ] 7.3 CSDN 密钥失效恢复路径演练：只改 `SECRET_KEY` 常量即可恢复
- [ ] 7.4 原 phase2 的注册表能力位（6.1 / 6.2 / 6.3）并入 6.1 / 6.2 / 6.7，确认无遗漏
- [ ] 7.5 原 phase2 的知乎直发项（1.2 / 1.3 / 1.4 / 5.1–5.6 / 6.4b / 6.5b）作废，由 5.1 的 Webview 方案取代，归档时在 `design.md` Decision 9 已有记录
- [ ] 7.6 原 phase2 的集成验证（7.3 / 7.4）并入 8.3 / 8.4

## 8. 集成验收

- [ ] 8.1 `cargo test` 全绿（新增通道与 16 个适配器单测）
- [ ] 8.2 `pnpm build`（core 先行）与 `pnpm tauri build` 通过
- [ ] 8.3 19 平台批量同步真机跑通：草稿与发布两条路径各一轮
- [ ] 8.4 断网场景：全部平台归类 `network_error`，不误报鉴权失败
- [ ] 8.5 部分成功场景：映射写入草稿标识，下次同步走更新而非重复建稿
- [ ] 8.6 导出/导入/重置：`webview` 平台不被当作 Cookie 解密

## 9. 文档与总纲同步

- [ ] 9.1 更新 `docs/快发布-OpenPub - 开源多平台内容分发中枢.md` 路线图：8 平台 → 19 平台，标注通道与批次
- [ ] 9.2 总纲记录通道分层、签名红线与 Webview 通道的维护成本
- [ ] 9.3 `design.md` 的 Open Questions 在实现过程中逐条关闭并回写结论