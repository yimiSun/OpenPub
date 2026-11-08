# Proposal

## Why

Phase 0 已交付可运行的 Tauri 桌面壳与 Monorepo 基础设施，并在实现过程中已经跑通了博客园（MetaWeblog XML-RPC）与掘金（REST 草稿接口）两条真实发布链路，同时补齐了 Windows DPAPI 加密存储。但这些能力目前是**"野生代码"**：它们直接写在 `src-tauri/src/lib.rs` 里，没有 spec 承载、没有统一路由、没有日志与错因归类，且前端 `PLATFORMS` 常量与 Rust `platform_publish` 的 match 分支需要人工双写，极易漂移。Phase 1 的目标是把这两条已验证的链路收编为正式能力，建立单一平台注册表与统一日志契约，为接入 CSDN 及后续平台铺平道路。

## What Changes

- 新增平台适配器注册表契约：单一来源声明平台元数据（id、展示名、凭据类型、登录 URL、能力开关），前端与 Rust 侧均由该注册表派生，消除双写漂移
- 新增统一发布请求契约：`platform_publish` 的入参、返回、错误模型规范化，区分「未配置」「凭据失效」「平台风控」「网络失败」「平台未实现」五类结果
- 新增同步日志契约：每次同步产出结构化日志条目（时间、平台、阶段、结果、可读错误、草稿链接），并支持导出
- 收编博客园适配器：MetaWeblog XML-RPC 链路正式纳入 `platform-adapters` 能力，明确必填配置与错误映射
- 收编掘金适配器：REST 草稿创建链路正式纳入，明确 Cookie 鉴权与 tag/category 处理
- 新增文章映射与发布状态存储（`sync_map`）：以**文章稳定 id**（本地生成的 UUID）为唯一键，记录每个平台的 `{ id, published }`，使「第二次同步」能够识别为更新而非新建；正文哈希降级为辅助校验
- 新增文章管理视图：左侧编辑器由「单一文本框」升级为「文章列表 + 当前文章」结构，支持新建文章与切换文章，切换或重启后可按文章 id 读回各平台映射
- 扩展发布命令契约：新增 `existing_id` / `is_update` / `publish_status` 三个参数，支持新建草稿、新建并发布、更新草稿、更新已发布四条路径
- 博客园适配器增加 `metaWeblog.editPost` 更新路径，并将 `newPost` 的 publish 布尔值由写死的 `0` 改为由 `publish_status` 驱动
- 掘金适配器增加草稿更新路径，并区分「更新草稿」与「更新已发布文章」两种接口；掘金「新建即发布」暂返回 `not_implemented` 并在前端给出可读提示
- 前端新增发布前风控确认：当 `publish_status = true` 时强制弹出不可逆覆盖提示，用户确认后方可发起请求
- 修改 `tauri-shell` 与 `auth-demo` 的既有需求：凭据读取/清除命令、DPAPI 解密链路写入正式能力
- **BREAKING**（语义）：原「仅创建草稿」约束收窄为默认行为，在用户显式选择覆盖时允许写入线上文章
- **BREAKING**（内部）：`src-tauri/src/lib.rs` 中内联的 `publish_cnblogs` / `publish_juejin` 将被移入独立模块 `src-tauri/src/platforms/`，`lib.rs` 仅保留命令注册与路由

## Capabilities

### New Capabilities

- `platform-adapters`: 平台适配器的注册表、统一发布路由与结果模型，覆盖博客园与掘金两条已跑通链路，并为后续平台提供一致接入点
- `sync-logging`: 同步日志的采集、结构化表达、三态 UI 呈现与导出能力
- `sync-mapping`: 文章稳定标识与平台映射存储，记录每篇文章在各平台的文章 id 与线上发布状态，支撑「更新而非新建」的同步语义
- `article-workspace`: 本地文章集合的创建、管理、当前文章切换与持久化，为同步映射提供稳定的文章身份

### Modified Capabilities

- `tauri-shell`: 新增「平台凭据读写命令」与「平台发布命令」两条需求；明确 `publish_*` 逻辑从 `lib.rs` 迁出至 `platforms/` 模块的组织约束
- `auth-demo`: 将「Cookie 加密存储」需求由示例性质升级为可复用契约，补充必须提供 `load_cookie` / `clear_cookie` 读写入口以及旧明文数据的迁移/拒绝策略

## Impact

- **受影响代码**
  - `apps/desktop-app/src-tauri/src/lib.rs`：拆分出 `platforms/mod.rs`、`platforms/cnblogs.rs`、`platforms/juejin.rs`
  - `apps/desktop-app/src-tauri/src/articles.rs`：新增文章集合的增删改查与持久化命令
  - `apps/desktop-app/src/App.tsx`：左侧编辑器重构为「文章列表 + 当前文章编辑区」；`PLATFORMS` 常量改为从共享注册表派生；日志区接入结构化日志
  - `packages/core/src/`：新增平台注册表、文章标识与日志条目的 TypeScript 类型契约
  - `packages/ui/src/components/`：`StatusBadge` 扩展为可表达五类同步结果；日志列表组件补齐 Loading / Empty / Error 三态
- **受影响 API / 依赖**
  - Tauri 命令：`platform_publish` 的入参结构调整为携带 `article_id`、`existing_id`、`is_update`、`publish_status` 与 `options`；新增 `list_platforms`、`list_articles`、`create_article`、`update_article`、`delete_article`、`load_cookie`、`clear_cookie`、`load_sync_map`、`save_sync_entry`、`clear_sync_entry`
  - 存储：新增 `articles` 集合与 `sync_map` 键空间，键名以文章 UUID 为唯一键
  - 无新增运行时依赖，UUID 使用现有依赖或轻量实现，沿用 `reqwest`、`serde`、`windows-sys`
- **风险**
  - 注册表单一来源改造涉及 Rust 与 TS 双端生成，若两侧手工维护仍会漂移，需要在 tasks 中约定以 TS 为权威并生成 Rust 侧的静态映射
  - 掘金接口依赖 Cookie 与风控，自动化测试难以覆盖，需以「错误归类正确」作为可验收标准而非「请求成功」
  - 掘金「更新已发布文章」接口地址（`article/update`）尚未经抓包验证，若与真实接口不符需按抓包结果修正
  - 左侧编辑器重构涉及现有 UI 结构变更，需保留 Markdown 输入与平台面板的既有交互
  - `publish_status = true` 会不可逆地覆盖线上文章，误触发代价高，必须由显式用户确认把关