# 🚀 快发布-OpenPub - 开源多平台内容分发中枢

> **"让天下没有难发的文章，绝不让创作者被割韭菜！"**
>
> 一款纯本地运行、代码完全开源、基础功能永久免费的跨端内容分发神器。

---

## 📖 一、项目背景与愿景

### 1.1 市场痛点

当前市面上的多平台发布工具（如 CSDN 同步助手、各类自媒体分发平台）普遍存在以下问题：

- **额度割韭菜**：核心同步功能被限制次数，强制订阅付费。
- **场景割裂**：网页端、客户端、开发工具之间无法互通，打断开发者心流。
- **黑盒风险**：闭源工具存在隐私泄露风险，且粗暴的“全自动发布”极易触发平台风控导致封号。
- **国内环境适配差**：国内平台接口封闭、外链排斥、富文本与图片防盗链机制复杂，普通工具难以完美兼容。

### 1.2 项目愿景

为开发者、极客与内容创作者，提供最纯粹、最高效、最安全的跨平台分发体验。

- **Local First**：数据与鉴权完全在本地流转，绝对不碰用户密码。
- **Geek Driven**：深度融入开发工作流，提供极客风的桌面应用体验。
- **Rule Compliant**：敬畏国内平台生态，以“安全草稿同步”为核心。
- **Open Source**：100% 开源，建立社区共建的适配器生态。

---

## ⚔️ 二、核心战略

### 2.1 形态战略：一核单翼，聚焦桌面

经过技术论证与决策，放弃早期的 VSCode/Chrome 插件形态，**全面聚焦于基于 Tauri v2 的桌面端应用**。

- **核心层（Core）**：纯 TypeScript 实现，剥离宿主环境依赖，负责 Markdown 解析、平台适配调度、状态机管理。
- **桌面翼（Tauri App）**：负责 UI 渲染（React）、本地数据持久化、跨平台系统调用。首发主力支持 Windows，后续适配 macOS。

### 2.2 发布战略：草稿优先，勾选直发，能力边界如实告知

- **国内平台**：默认执行“一键全网建草稿”。**若用户在界面上勾选了“覆盖已发布的文章”，则调用相应平台的更新/发布接口，直接覆盖线上文章。**
- **能力边界（当前实现）**：18 个平台中 **10 个支持真实发布**（博客园、CSDN、知乎、思否、小红书图文、51CTO、开源中国、腾讯云、微博、头条），**8 个仅支持草稿**（掘金、微信公众号、阿里云、百家号、网易号、哔哩哔哩、抖音、简书）。
- **绝不静默降级**：仅草稿平台被勾选发布时，适配器返回 `not_implemented` 并明确提示「该平台暂不支持直接发布，请在平台后台手动发布」，`is_published` 如实为 `false`；不会悄悄存草稿再报成功。
- **海外平台**：依托官方 API（Dev.to, Hashnode, Medium），支持直接发布。同时内置 AI 出海翻译（BYOK），一键生成英文长文。

### 2.3 迭代战略：v1.0.0 一次性平推国内 18 平台

- **v1.0.0**：不切批次，一次性覆盖国内 18 个平台（技术开发者基本盘 + 泛媒体），见第五章路线图。
- **通道分层**：注册表 `channelType` 只有两种取值——
  - `direct`（Rust 直发，12 个）：凭据为 Cookie + 静态请求头。需要页面 token、响应头 CSRF 或身份标识的平台（掘金、微信公众号、微博、网易号、哔哩哔哩）在业务请求前多做一次预检，预检属于该通道内部步骤，不单列通道。
  - `webview`（网页模拟，6 个）：在平台自己的页面里执行平台自身的 JS，只搬运文章数据与结果，不逆向签名。知乎、简书、思否、小红书图文、头条、抖音。
  - 博客园的 MetaWeblog XML-RPC 由 `credentialKind: "metaweblog"` 表达，`channelType` 仍为 `direct`。
- **后续扩展**：海外平台（Dev.to / Hashnode / Medium）留给 V1.1.0；抖音图文与小红书长文笔记留待后续版本，并提供标准化适配器接口，通过开源社区众包补齐更多平台。

---

## 🏗️ 三、技术架构与规范

### 3.1 Monorepo 目录结构（基于 pnpm workspaces）

```text
openpub/
├── packages/
│   ├── core/                 # 共享层：平台注册表、类型、同步映射、常量
│   ├── ui/                   # 共用 UI 组件（React + Tailwind）
│   └── adapters/             # 脚手架生成的适配器包（社区 PR 入口，当前含 cnblogs）
├── apps/
│   └── desktop-app/          # Tauri 壳（Rust + React）
│       ├── src-tauri/        # Rust 后端：平台适配器、登录窗口、同步编排、DPAPI 加密
│       ├── src/              # React 前端 UI
│       └── tauri.conf.json   # Tauri 配置
├── docs/                     # 项目总纲与参考实现（index.mjs / index2.mjs）
├── openspec/                 # OpenSpec 变更制品
└── package.json
```

> 说明：v1.0.0 的 18 个平台适配器全部实现在 `apps/desktop-app/src-tauri/src/platforms/` 下（`direct` 通道平铺、`webview` 通道在 `webview/` 子目录），未按平台拆成独立 workspace 包——18 个平台共享同一套 `shared/` 工具与测试夹具，拆包只会增加跨包改动成本。`packages/adapters/` 保留为社区贡献新平台的脚手架位置。

### 3.2 核心技术栈

| 类别 | 选型 |
| --- | --- |
| 语言 | TypeScript（前端 / Core）+ Rust（Tauri 后端） |
| 桌面框架 | Tauri v2（轻量、系统原生调用、WebView2 通道复用登录态） |
| 构建工具 | Vite + @tauri-apps/cli，pnpm workspaces |
| Markdown 渲染 | 前端预览 `react-markdown` + `remark-gfm`；提交到平台的正文由 Rust `shared/markdown.rs` 转 HTML（转义 + 段落包裹，刻意保持最小实现，避免各平台编辑器二次解析差异） |
| UI 框架 | React + Tailwind CSS（暗黑模式优先） |
| 凭据存储 | Windows DPAPI（`windows-sys`），仅 `direct` 通道的 Cookie 走此路径 |

### 3.3 核心接口规范

前端注册表 `PlatformDefinition`（`packages/core/src/platforms/registry.ts`）是平台元数据的唯一来源，每个平台声明两个正交维度：

- `credentialKind`：凭据怎么收——`cookie`（内嵌登录窗口扫码或手动粘贴）/ `webview`（通道窗口自己持有登录态）/ `metaweblog`（MetaWeblog Token）。
- `channelType`：请求怎么发——`direct`（Rust 直发，含页面 token / CSRF / 身份预检）/ `webview`（平台页面内执行）。

```typescript
export interface PlatformDefinition {
  id: string;
  name: string;
  credentialKind: "cookie" | "webview" | "metaweblog";
  loginUrl: string;
  requiredCookieKeys: readonly string[];
  requiredConfigKeys?: readonly string[];  // 当前仅微信公众号需要 token
  region: "domestic" | "overseas";
  channelType: "direct" | "webview";
  available: boolean;                       // 真机验证通过后才置 true
  loginWindow?: boolean;                    // webview 平台是否有独立登录窗口
  manualCookieOnly?: boolean;               // 兜底开关，当前无平台使用
}
```

发布侧统一使用两个结构，接入新平台不需要改它们：

```rust
pub struct PublishRequest {
    pub title: String,
    pub content: String,
    pub existing_id: Option<String>,
    pub is_update: bool,
    pub publish_status: bool,
    pub existing_published: bool,
    pub tags: Option<String>,              // 逗号分隔，None 表示用户未填
    pub existing_draft_id: Option<String>, // 上次草稿模式的远端草稿 ID
}

pub struct PublishResult {
    pub success: bool,
    pub post_id: String,
    pub edit_url: String,
    pub message: String,
    pub failure_class: Option<FailureClass>, // not_configured / auth_invalid / platform_error / network_error / not_implemented / unknown
    pub operation: PublishOperation,         // 草稿 / 发布 / 更新
    pub is_published: bool,                  // 如实反映，草稿即 false
    pub draft_id: Option<String>,            // 草稿模式下必须记住，否则下次会重复建稿
}
```

平台能力边界由适配器行为表达，不在注册表另立 `publishCapability` 字段——注册表是声明、适配器是事实，两者分离必然漂移。

### 3.4 共享层

`platforms/shared/` 提供 `direct` 通道的公共能力：

- `page_token.rs`：从页面 HTML 抽取 token（微信公众号 `token`、微博 `config` JSON 字段）。
- `csrf.rs`：`probe_csrf_token` 走 HEAD 请求读响应头 `x-ware-csrf-token`；`x-secsdk-csrf-version` 由调用方传入（掘金 `1.2.10`），取值方式（整串 / 逗号第 N 段）作为入参。
- `cookie.rs` / `markdown.rs` / `truncate.rs` / `extract.rs`：Cookie 解析、HTML 转换、截断与字段抽取。

预检失败一律归类 `not_configured`，且在发业务请求前终止——用请求计数断言，而非只看返回码。

---

## 🍏 四、跨平台适配规范（macOS）

首发版验证 Windows 平台后，将开启 macOS 适配。由于采用 Tauri 架构，核心业务代码无需重写，仅需处理系统底层差异。

### 4.1 核心差异与应对策略

- **加密存储（必须改）**
  - Windows 采用 DPAPI（通过 `windows-sys` 调用）。
  - macOS 采用 Keychain（钥匙串）。
  - 策略：在 Rust 侧通过 `#[cfg(target_os = "windows")]` 进行条件编译隔离，或者直接引入跨平台的 `keyring` 库，自动适配两端。
- **Webview 内核差异（前端兼容）**
  - Windows 使用 WebView2 (Chromium)，macOS 使用 WKWebView (WebKit)。
  - 策略：前端代码遵循严格的 Web 标准，警惕文件拖拽、剪贴板读取等原生行为在不同内核下的限制。
- **绝对路径处理**
  - 策略：严禁在 Rust 中硬编码 `C:\`，统一使用 Tauri 的 `app.path()` 或 `std::path::PathBuf` 获取跨平台路径。

### 4.2 分发与签名策略

- **Windows**：直接输出 `.msi` 或 `.exe`。
- **macOS**：若不走 Apple Store（节省 $99/年），将采用 Ad-hoc 签名 + 引导用户手动放行，或通过 Homebrew Cask 进行分发，以规避 Gatekeeper 的拦截。

---

## 🎯 五、平台路线图 (Roadmap)

### 🟢 V1.0.0 首发版：国内 18 平台一次性平推

v1.0.0 不切分批次，一次性覆盖国内全部 18 个平台。当前 18 个适配器全部落地，17 个已通过真机验证；思否因缺测试账号待验。

两条执行通道（`channelType`）：

- **`direct`（12 个）**：Rust 直发，凭据为 Cookie + 静态请求头。需要先取页面 token 或响应头 CSRF 的平台（掘金、微信公众号、微博、网易号、哔哩哔哩）仍属此通道，差异体现为适配器内部的 preflight（由 `shared/page_token.rs` 与 `shared/csrf.rs` 提供），不单列通道。博客园的 MetaWeblog XML-RPC 由 `credentialKind: "metaweblog"` 承载，`channelType` 同为 `direct`。
- **`webview`（6 个）**：在平台自己的页面里执行平台自己的 JS，只搬运文章数据与结果，**不移植任何签名算法**。

| 平台 | 通道 | 能力 | 关键依赖 / 预案 | 状态 |
| --- | --- | --- | --- | --- |
| 博客园 | `direct`（MetaWeblog） | 真实发布 | 古老但稳定的 XML-RPC，纯服务端推送 | 已交付，真机通过 |
| 掘金 | `direct` | 仅草稿 | 需页面 token 预检（`x-ware-csrf-token` 探针，须带 `x-secsdk-csrf-request: 1`）；发布页有额外业务校验，不补直发 | 已交付，真机通过 |
| CSDN | `direct` | 真实发布 | 阿里云网关 HMAC-SHA256（唯一授权签名例外），发布必须带标签 | 已交付，真机通过 |
| 微信公众号 | `direct` | 仅草稿（纯文字） | 需页面 token 预检（token 在后台 URL 而非 Cookie）；`requiredConfigKeys: ["token"]` | 已交付，真机通过 |
| 简书 | `webview` | 仅草稿 | 笔记 + 发布两步，图片走七牛上传 token；直发被风控拒绝后改判网页模拟 | 已交付，真机通过 |
| 51CTO | `direct` | 真实发布 | 博客后台发布接口 | 已交付，真机通过 |
| 开源中国 | `direct` | 真实发布 | `apiv1.oschina.net` 草稿/发布 | 已交付，真机通过 |
| 腾讯云开发者社区 | `direct` | 真实发布 | `cloud.tencent.com/developer/api/*` | 已交付，真机通过 |
| 阿里云开发者社区 | `direct` | 仅草稿 | 需带 CSRF 头；图片上传单独处理 | 已交付，真机通过 |
| 百家号 | `direct` | 仅草稿 | `pcui/article/save` + 素材上传 | 已交付，真机通过 |
| 网易号 | `direct` | 仅草稿 | `mp.163.com` 表单 POST | 已交付，真机通过 |
| 微博 | `direct` | 真实发布 | 需页面 token 预检（从编辑器页面配置 JSON 抽凭据） | 已交付，真机通过 |
| 哔哩哔哩 | `direct` | 仅草稿 | Cookie 内自带 `bili_jct` 作为 CSRF | 已交付，真机通过 |
| 头条 | `webview` | 真实发布 | 直发被风控拒绝，改判网页模拟 | 已交付，真机通过 |
| 知乎 | `webview` | 真实发布 | 全站双层 `x-zse-93/96` + `x-zst-81` + `ZP` 函数，**不逆向** | 已交付，真机通过 |
| 小红书（图文笔记） | `webview` | 真实发布 | `_webmsxyw` / `X-s-common` 运行时签名，**不逆向** | 已交付，真机通过 |
| 抖音 | `webview` | 仅草稿 | 创作者中心运行时校验，**不逆向** | 已交付，真机通过 |
| 思否 | `webview` | 真实发布 | gateway `keyv` / `ivd` 页面内加密串，**不逆向** | 已实现，待真机验证（暂无账号） |

**不在 v1.0.0 范围**：抖音图文（发布接口另需 `media_type`、封面素材与长文分支参数）、小红书长文笔记。

**签名红线**：凡契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway 加密串 / 依赖页面脚本执行的平台，一律走 `webview` 通道，不做逆向。唯一例外是 CSDN（已获授权、密钥已知、单层 HMAC）。

**维护成本须知**：`webview` 通道的 6 份页面脚本依赖平台页面结构，平台改版即失效；CSDN 网关密钥在 CSDN 前端 JS 里，CSDN 换密钥时本应用会静默失效。两者都是这条路径的固有成本，不是回归 bug。

**交付约束**：先草稿、后发布；绝不静默降级（勾选发布就必须真发布，做不到就返回 `not_implemented` 并提示手动发布）；部分成功仍写入映射，占位 ID 不落盘；`available` 只有真机验证通过才置 `true`。

### 🟡 V1.1.0 拓荒版（海外）

- **海外平台**：Dev.to, Hashnode, Medium。内置大模型（BYOK）实现中英双语一键出海。
- 国内 18 平台在 v1.0.0 已全部覆盖，V1.1.0 不再追加国内平台，只做海外通道与多语言。

### 🔵 V1.2.0+ 生态版与多端

- **macOS 适配**：完成底层差异隔离，支持 macOS 端构建与签名分发。
- 引入社区众包机制，开放 `CONTRIBUTING.md`，设立“悬赏榜”鼓励开发者提交 PR。

---

## 💰 六、开源生态与商业化 (Open Core)

坚决抵制破坏体验的弹窗广告与强制打赏。采用业界成熟的开放核心 (Open Core) 模式：

- **社区版 (Free)**：永久免费。包含核心平台同步、基础 Markdown 解析、有限次数的 AI 出海翻译。不嵌任何广告。
- **专业版 (Pro)**：按年/月订阅（如 ¥19/月）。包含：无限 AI 翻译（用户自带 API Key）、多账号矩阵管理、同步失败自动重试、完整同步历史记录、高级图片处理。
- **企业版 (Enterprise)**：定制化私有部署，团队许可证统一管理。
- **收款方式**：通过外部独立站（Stripe / 聚合支付）完成收款，发放 License Key，软件内验证解锁。

---

## 🛡️ 七、免责声明与红线（必须置于 README 醒目位置）

- **数据隐私**：本软件完全在本地运行，所有平台 Cookie、Token 及文章数据绝不经过第三方服务器中转，代码开源可查。
- **网络免责**：本项目不提供任何形式的网络代理（翻墙）服务。海外平台与 AI 接口的连通性，依赖于用户本地的网络环境。
- **发布控制**：默认仅提交至“草稿箱”，是否直接发布由用户逐篇勾选决定；能力边界如实告知——当前 10 个平台支持真实发布，8 个平台仅支持草稿，勾选发布而平台不支持时返回 `not_implemented` 并提示手动发布，不做静默降级。因用户直接发布或频繁操作引发的平台限制/封号，本项目不承担相关责任。

---

## 👨‍💻 八、协作与贡献指南

- **开发规范**：遵循 Conventional Commits 提交规范（如 `feat: 新增掘金适配器`）。
- **CI/CD**：GitHub Actions 自动构建 Windows 安装包及 macOS 安装包。
- **适配器开发**：提供 `create-adapter` 脚手架，开发者可一键生成模板，专注编写 `sync` 逻辑。