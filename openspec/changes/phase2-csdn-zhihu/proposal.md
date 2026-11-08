# Proposal

## Why

Phase 1 已把「草稿优先 + 再次同步走更新而非新建」的引擎跑通并归档：博客园与掘金两条链路实测通过，注册表、路由决策、映射回填、错因归类都已成型。但当前引擎只有两个样本，**无法证明它是可复制的**——它可能只是恰好适配了这两家。

Phase 2 接入 CSDN 与知乎，正是对架构底座的压力测试：这两家引入掘金没有的新问题（CSRF 双因子鉴权、建稿与发布是两个独立动作），如果接入它们仍不需要改动核心路由逻辑，底座才算真正锁死。同时，掘金私有实现的 Cookie 读取、Markdown→HTML 转换、错误响应截断必须上提为共享工具，否则三份实现会立刻开始漂移。

## What Changes

- 新增平台共享工具能力：把目前私有在 `juejin.rs` 里的 Cookie 读取、Markdown→HTML 转换、错误响应按字符截断上提为三平台共用契约，并新增 CSRF token 提取（CSDN `x-csrf-token`、知乎 `_xsrf` 含 URL-decode）
- 收编 CSDN 适配器：Cookie + REST，复用掘金的 `existing_published` 路由决策；草稿保存与公开发布为两个端点
- 收编知乎适配器：Cookie + XSRF，`POST /api/v4/articles` 建稿，公开发布为**第二个独立动作**，适配器内部两段式、对外仍是单一 create/update 语义
- 注册表新增「发布能力位」：声明每个平台是否支持线上发布、是否仅支持草稿，前端提示由能力位驱动，移除 `selectedPlatforms.has("juejin")` 这类平台 id 硬编码
- 映射写入补充「两段式部分成功」语义：知乎建稿成功但发布失败时，仍写入 `published: false` 的草稿映射，使用户下次同步能续上而不是产生重复文章
- 同步日志新增 `publish` 阶段：区分「建稿阶段失败」与「发布阶段失败」，让两段式流程的失败可定位
- 注册表把 `csdn` / `zhihu` 的 `available` 由 `false` 置为 `true`，并把两者加入已实现适配器清单
- **BREAKING**（内部）：`juejin.rs` 中 `escape_html` / `to_html_content` / `truncate_chars` / Cookie 读取被移除并改为调用共享模块；掘金行为不变，由既有单测守护
- **BREAKING**（内部）：`platforms/mod.rs` 的 `dispatch` 新增 `csdn` / `zhihu` 分支，`SUPPORTED_PLATFORM_IDS` 与 TS 注册表的一致性校验范围随之扩大

## Capabilities

### New Capabilities

- `platform-shared-utils`: 所有 Cookie 类平台适配器共用的凭据与请求工具契约——从 Cookie 串提取指定键值（缺失时明确报错而非空串兜底）、Markdown→HTML 转换（转义 + 段落包装）、CSRF/XSRF token 提取（含 URL-decode）、错误响应按字符截断

### Modified Capabilities

- `platform-adapters`: 新增 CSDN 适配器需求（草稿保存与发布端点分离、`existing_published` 路由）；新增知乎适配器需求（建稿与发布两段式、`is_published` 语义映射）；新增「CSRF 双因子鉴权」需求；新增「Markdown→HTML 转换统一走共享实现」需求；注册表需求扩展为「声明发布能力位」且 `csdn` / `zhihu` 转为可用
- `sync-mapping`: 新增「两段式部分成功仍写入草稿映射」需求，明确建稿成功但发布失败时的映射写入行为与后续续写语义
- `sync-logging`: 日志阶段枚举新增 `publish`，用于区分建稿阶段与发布阶段的失败

## Impact

- **受影响代码**
  - `apps/desktop-app/src-tauri/src/platforms/`：新增 `shared/`（cookie、markdown、csrf、truncate）或 `platforms/shared.rs` 拆分为多个模块；新增 `platforms/csdn.rs`、`platforms/zhihu.rs`；`juejin.rs` 改为调用共享实现；`mod.rs` 扩展 `dispatch` 与 `SUPPORTED_PLATFORM_IDS`
  - `apps/desktop-app/src-tauri/src/lib.rs`：`platform_publish` 的两段式结果回填逻辑需支持「部分成功」语义（建稿成功、发布失败）
  - `apps/desktop-app/src/App.tsx`：移除掘金硬编码提示，改为按注册表能力位渲染；CSDN / 知乎复用既有 Cookie 配置弹窗
  - `packages/core/src/platforms/registry.ts`：新增能力位字段；`ADAPTER_IMPLEMENTED_IDS` 加入 `csdn` / `zhihu`；`available` 置 `true`；`auditRegistry()` 扩展校验能力位与已实现清单的一致性
  - `packages/core/src/platforms/sync.ts`：日志阶段类型加入 `publish`
- **受影响 API / 依赖**
  - Tauri 命令签名不变（`platform_publish` 入参已覆盖 Phase 2 所需），无新增命令
  - 无新增运行时依赖：Markdown→HTML 继续沿用现有轻量实现，不引入 `pulldown-cmark` 等渲染库
  - 凭据存储沿用既有 DPAPI 加密的 `{platform}_cookie` 键空间，CSDN 与知乎无需新增存储结构
- **风险**
  - CSDN 与知乎的接口契约均未经抓包验证，字段名与端点存在与掘金同类的「写错就静默失败」风险，必须先完成契约验证再写实现
  - 知乎两段式流程中「建稿成功、发布失败」是新增的中间态，若映射写入时机处理不当会产生重复草稿
  - 知乎 `_xsrf` 需 URL-decode 后才能使用，遗漏解码会表现为看似成功的 403
  - CSDN 风控对请求频率与 Referer 较敏感，失败需归入 `platform_error` 而非静默重试
  - 共享工具上提涉及掘金既有行为，重构若引入偏差会破坏已实测通过的链路，必须以掘金现有 8 个单测作为回归门槛