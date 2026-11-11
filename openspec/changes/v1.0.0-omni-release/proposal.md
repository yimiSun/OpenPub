# Proposal

## Why

v1.0.0 只有一条主线：**把国内 18 个平台一次性平推上线**。此前的 phase2（CSDN / 知乎）与 phase3（产品体验）是两条并行的局部变更，各自有价值，但没有一个「v1.0.0 到底交付什么、按什么顺序、做到哪一步算完成」的单一事实来源。本变更把三者归集为**唯一的总流程**，phase2 / phase3 不再单独推进。

支撑这个决定的事实有三条：

1. **参考实现已经把契约写死了**。`docs/index.mjs`（19 个平台）与 `docs/index2.mjs`（18 个平台，缺网易号）是一份完整可运行的适配器实现，含每个平台的 `checkAuth` / `saveDraft` / `publishArticle`、`HEADER_RULES`、CSRF 获取方式与页面注入脚本。端点、字段名、token 来源都不需要再逐平台抓包摸索。
2. **参考实现同时暴露了红线**。19 个平台里有 7 个依赖页面运行时签名（知乎 `x-zse-93`/`x-zse-96`/`x-zst-81` 与 `ZP` 函数、小红书 `_webmsxyw`/`X-s-common`、思否 gateway `keyv`/`ivd` 加密串、抖音），另有 8 个依赖 `runtime.tabs.executeScript` 在平台页面里执行脚本。把这些搬到 Rust 侧等于逆向签名，触碰项目红线。
3. **现状缺口明确**（变更发起时）：注册表只登记 9 个平台、已实现适配器 4 个；掘金仅支持草稿；微信公众号仅纯文字草稿；没有「在平台页面里执行脚本」的执行通道（现有 Webview 只用来登录取 Cookie）。

## What Changes

- **新增执行通道分层**：每个平台适配器在注册表声明 `channelType`，枚举只有两种——
  - `direct`：Rust 直发，凭据为 Cookie + 静态头（含 Cookie 里已有的 CSRF，如 `bili_jct`）。需要先取页面 token / 响应头 CSRF / 身份标识的平台仍属此通道，差异体现为适配器内部的 preflight（由 `shared/page_token.rs` 与 `shared/csrf.rs` 提供），不单列通道。
  - `webview`：在平台自己的页面里执行平台自己的 JS，只搬运 payload 与结果，**不移植任何签名算法**。
  - 博客园的 MetaWeblog XML-RPC 直发由 `credentialKind: "metaweblog"` 表达，`channelType` 仍为 `direct`。
- **新增 Webview 注入通道**：受控 `WebviewWindow`（固定 label `channel-{platform}`、复用登录态）+ 页面脚本 + IPC 回传结构化结果，含超时、失败分类、`postId`/`editUrl` 回传。
- **新增 14 个平台适配器**，最终通道分布为 `direct` 12 个 / `webview` 6 个：
  - `direct` 新增 8 个：51CTO、开源中国、腾讯云、阿里云、百家号、网易号、哔哩哔哩、微博；原已交付的博客园、掘金、CSDN、微信公众号维持此通道（其中掘金、微信公众号补页面 token 预检）。
  - `webview` 6 个：知乎、小红书图文、抖音、思否，以及真机复测后由 `direct` 改判的简书与头条。
- **注册表重构**：id 与参考实现 bundle 对齐（`tencent` → `tencentcloud`），补齐 18 条，新增 `channelType`，`available` 改为「真机验证通过才置 `true`」。不引入 `publishCapability`，能力由适配器实际行为表达。
- **双清单按通道重划**：`webview` 平台不进 `LOGIN_SUPPORTED_PLATFORMS` / `COOKIE_CREDENTIAL_PLATFORMS`，其登录态由适配器自己的 Webview 持有；`direct` 平台维持现有 Cookie 路径。
- **归集 phase2 / phase3**：
  - phase2 已完成的 CSDN 草稿 + 直发 + 网关签名、共享工具上提、两段式部分成功映射，作为**已交付基线**并入本变更，不再单列。
  - phase2 的知乎 Cookie 直发方案**作废**，由 Webview 通道承接；phase2 未完成的 4 项（CSDN 守卫复活、CSDN 草稿更新、注册表能力位、集成验证）**并入本变更**继续推进并已完成。
  - phase3 的 42 项（关于/支持、一键登录、设置中心、版本检测、启动白屏）**全部完成**，作为基线并入，其 spec delta 一并归集到本变更。

## Capabilities

### New Capabilities

- `webview-injection-channel`: 在受控 `WebviewWindow` 内执行平台页面脚本并回传结构化结果的执行通道——窗口生命周期与 label 复用、页面就绪判定、payload 注入、IPC 回传协议、超时与失败分类，以及「不得移植签名算法」的硬约束。
- `app-settings`: 全局设置中心与配置导出/导入/重置（归集自 phase3，已完成）。
- `support-about`: 关于/支持弹窗与推广区（归集自 phase3，已完成）。
- `update-check`: 版本检测与引导下载（归集自 phase3，已完成）。

### Modified Capabilities

- `platform-adapters`: 注册表由 9 条扩到 18 条并新增 `channelType` 能力位；新增 14 个适配器行为契约；平台能力边界（真实发布 / 仅草稿）以适配器返回值表达；全部沿用「先草稿后发布、绝不静默降级、占位 ID 不落盘」的既有约束。
- `platform-shared-utils`: 新增页面 token 抽取与 `x-secsdk` 风格 CSRF 探针，作为 `direct` 通道的 preflight 能力复用。
- `auth-demo`: `webview` 通道平台的登录态由适配器 Webview 持有，不写 DPAPI Cookie；`direct` 通道维持现状（归集自 phase3 的「按平台选择登录页 / Cookie 完整性校验 / 登录成功后前端刷新」三条一并生效）。
- `tauri-shell`: 新增 Webview 通道窗口的 capability（含远程域名 IPC 白名单），与 phase3 归集的剪贴板、宿主信息、登录窗口 capability 并列。

## Impact

- 前端：`packages/core/src/platforms/registry.ts`（18 条 + `channelType`）、`App.tsx`（按 `channelType` 渲染凭据入口与提示）。
- 后端：`platforms/mod.rs`（`dispatch` 新增 14 分支）、新增 `platforms/webview/`（通道实现 + 6 份页面脚本）、新增 `platforms/shared/page_token.rs`、14 个新平台模块。
- 权限：新增 `capabilities/webview-channel.json`，为实际需要的平台域名声明 `remote.urls` 以允许远程页面回传 IPC。
- 双清单：`login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`、`settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS` 按通道重新划分。
- 变更目录：原 phase2 与 phase3 两个并行变更已归集进本变更并删除，避免出现第二份事实来源。
- 维护成本：Webview 通道的 6 份页面脚本依赖平台页面结构，平台改版即失效——与 CSDN 密钥同类，是这条路径的固有成本，不是回归 bug。
- 能力边界：8 个平台仅支持草稿（掘金、微信公众号、阿里云、百家号、网易号、哔哩哔哩、抖音、简书），勾选发布时如实返回 `not_implemented` 并提示手动发布。