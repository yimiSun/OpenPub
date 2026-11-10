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

### 2.2 发布战略：草稿优先，勾选直发

- **国内平台**：默认执行“一键全网建草稿”。**若用户在界面上勾选了“覆盖已发布的文章”，则调用相应平台的更新/发布接口，直接覆盖线上文章。**（当前进度：掘金已支持草稿更新；CSDN 已支持草稿保存与直接发布，实测发布后直接进入平台「已发布」列表，不经审核队列）
- **海外平台**：依托官方 API（Dev.to, Hashnode, Medium），支持直接发布。同时内置 AI 出海翻译（BYOK），一键生成英文长文。

### 2.3 迭代战略：v1.0.0 一次性平推国内 19 平台

- **v1.0.0**：不切批次，一次性覆盖国内全部 19 个平台（技术开发者基本盘 + 泛媒体），见第五章路线图。
- **通道分层**：能在服务端直发的走 `http`（页面 token / CSRF 预检属于该通道内部步骤）；需要运行时签名的走 `webview`，在平台页面内执行其自身 JS，不逆向；博客园走 `metaweblog`。
- **后续扩展**：海外平台（Dev.to / Hashnode / Medium）留给 V1.1.0，并提供标准化适配器接口，通过开源社区众包补齐更多平台。

---

## 🏗️ 三、技术架构与规范

### 3.1 Monorepo 目录结构（基于 pnpm workspaces）

```text
openpub/
├── packages/
│   ├── core/                 # 纯逻辑：Markdown解析、Adapter接口定义、图片处理
│   ├── ui/                   # 共用的 Webview UI 组件 (React + Tailwind)
│   └── adapters/             # 各大平台的适配器实现 (独立目录，方便社区PR)
├── apps/
│   └── desktop-app/          # Tauri 壳 (Rust + React)
│       ├── src-tauri/        # Rust 后端逻辑 (系统调用、网络请求、DPAPI加密)
│       ├── src/              # React 前端 UI
│       └── tauri.conf.json   # Tauri 配置文件
├── docs/                     # 开发文档与适配器编写指南
└── package.json
```

### 3.2 核心技术栈

| 类别 | 选型 |
| --- | --- |
| 语言 | TypeScript (前端/Core) + Rust (Tauri 后端) |
| 桌面框架 | Tauri v2 (极致轻量、安全、系统原生调用) |
| 构建工具 | Vite + @tauri-apps/cli |
| 解析引擎 | remark / turndown (Markdown 与 HTML 互相转换，处理代码高亮) |
| UI 框架 | React + Tailwind CSS (打造现代极客风，暗黑模式优先) |

### 3.3 核心接口规范：PlatformAdapter

所有平台适配器必须实现此接口（在 Rust 侧或 Core 层），以保证架构的可维护性与社区扩展性。

```typescript
export interface PublishOptions {
  title: string;
  markdown: string;
  tags?: string[];
  coverImage?: string;
  isDraftOnly: boolean;
}

export interface SyncResult {
  success: boolean;
  platform: string;
  draftUrl?: string;
  errorMsg?: string;
}

export interface PlatformAdapter {
  readonly id: string;
  readonly name: string;
  readonly env: ("browser" | "api")[];
  // 执行通道：直发（含页面 token / CSRF 预检）/ 页面内注入 / MetaWeblog
  readonly adapterChannel: "http" | "webview" | "metaweblog";

  // 1. 检查登录态 (Cookie 或 Token)
  checkAuth(): Promise<boolean>;

  // 2. 预处理 (可选，如清理特定平台不支持的 HTML 标签)
  preprocess?(options: PublishOptions): Promise<PublishOptions>;

  // 3. 图片同步机制 (转存图床，解决防盗链)
  uploadImages(images: string[], context: any): Promise<string[]>;

  // 4. 核心同步动作
  sync(options: PublishOptions, context: any): Promise<SyncResult>;
}
```

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

### 🟢 V1.0.0 首发版：国内 19 平台一次性平推

v1.0.0 不切分批次，一次性覆盖国内全部 19 个平台。

三条执行通道：

- **`http`**：Rust 直发，凭据为 Cookie + 静态请求头。需要先取页面 token 或响应头 CSRF 的平台仍属此通道，差异体现为适配器内部的 preflight（由 `shared/page_token` 与 `shared/csrf` 提供），不单列通道。
- **`webview`**：在平台自己的页面里执行平台自己的 JS，只搬运文章数据与结果，**不移植任何签名算法**。
- **`metaweblog`**：MetaWeblog XML-RPC 直发（博客园）。

| 平台 | 通道 | 关键依赖 / 预案 | 状态 |
| --- | --- | --- | --- |
| 博客园 | MetaWeblog | 古老但稳定的 XML-RPC，纯服务端推送 | 已交付 |
| 掘金 | `http` | 需页面 token 预检（`x-ware-csrf-token` 探针，须带 `x-secsdk-csrf-request: 1`）；补 `article/publish` 直发 | 草稿已交付，补直发 |
| CSDN | `http` | 阿里云网关 HMAC-SHA256（唯一授权签名例外），发布必须带标签 | 已交付 |
| 微信公众号 | `http` | 需页面 token 预检（token 在后台 URL 而非 Cookie）；纯文字草稿 | 草稿已交付 |
| 简书 | `http` | 笔记 + 发布两步，图片走七牛上传 token | 待接入 |
| 51CTO | `http` | 博客后台发布接口 | 待接入 |
| 开源中国 | `http` | `apiv1.oschina.net` 草稿/发布 | 待接入 |
| 腾讯云开发者社区 | `http` | `cloud.tencent.com/developer/api/*` | 待接入 |
| 阿里云开发者社区 | `http` | 需带 CSRF 头；图片上传单独处理 | 待接入 |
| 百家号 | `http` | `pcui/article/save` + 素材上传 | 待接入 |
| 网易号 | `http` | `mp.163.com` 表单 POST | 待接入 |
| 微博 | `http` | 需页面 token 预检（从编辑器页面配置 JSON 抽凭据） | 待接入 |
| 头条 | `http` | 需页面 token 预检（`x-ware-csrf-token` 探针，须带 `x-secsdk-csrf-request: 1`） | 待接入 |
| 哔哩哔哩 | `http` | Cookie 内自带 `bili_jct` 作为 CSRF | 待接入 |
| 知乎 | `webview` | 全站双层 `x-zse-93/96` + `x-zst-81` + `ZP` 函数，**不逆向** | 待接入 |
| 小红书长文笔记 | `webview` | `_webmsxyw` / `X-s-common` 运行时签名，**不逆向** | 待接入 |
| 小红书图文笔记 | `webview` | 同上，图文一并提交 | 待接入 |
| 抖音长文 | `webview` | 创作者中心运行时校验，**不逆向** | 待接入 |
| 抖音图文 | `webview` | 同上，`/web_api/sns/v2/note` | 待接入 |
| 思否 | `webview` | gateway `keyv` / `ivd` 页面内加密串，**不逆向** | 待接入 |

**签名红线**：凡契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway 加密串 / 依赖页面脚本执行的平台，一律走 `webview` 通道，不做逆向。唯一例外是 CSDN（已获授权、密钥已知、单层 HMAC）。

**维护成本须知**：`webview` 通道的 6 份页面脚本依赖平台页面结构，平台改版即失效；CSDN 网关密钥在 CSDN 前端 JS 里，CSDN 换密钥时本应用会静默失效。两者都是这条路径的固有成本，不是回归 bug。

**交付约束**：先草稿、后发布；绝不静默降级（勾选发布就必须真发布，做不到就报错）；部分成功仍写入映射，占位 ID 不落盘；`available` 只有真机验证通过才置 `true`。

### 🟡 V1.1.0 拓荒版（海外）

- **海外平台**：Dev.to, Hashnode, Medium。内置大模型（BYOK）实现中英双语一键出海。
- 国内 19 平台在 v1.0.0 已全部覆盖，V1.1.0 不再追加国内平台，只做海外通道与多语言。

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
- **发布控制**：国内平台默认仅提交至“草稿箱”，是否直接发布由用户逐篇勾选决定；应用不做静默降级（勾选发布而平台不支持时会明确报错）。因用户直接发布或频繁操作引发的平台限制/封号，本项目不承担相关责任。

---

## 👨‍💻 八、协作与贡献指南

- **开发规范**：遵循 Conventional Commits 提交规范（如 `feat: 新增掘金适配器`）。
- **CI/CD**：GitHub Actions 自动构建 Windows 安装包及 macOS 安装包。
- **适配器开发**：提供 `create-adapter` 脚手架，开发者可一键生成模板，专注编写 `sync` 逻辑。