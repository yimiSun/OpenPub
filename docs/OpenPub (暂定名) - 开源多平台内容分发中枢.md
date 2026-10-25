# 🚀 快发布-OpenPub - 开源多平台内容分发中枢

> **"让天下没有难发的文章，绝不让创作者被割韭菜！"**
>
> 一款纯本地运行、代码完全开源、基础功能永久免费的跨端内容分发神器。

---

## 📖 一、项目背景与愿景

### 1.1 市场痛点

当前市面上的多平台发布工具（如 CSDN 同步助手、各类自媒体分发平台）普遍存在以下问题：

- **额度割韭菜**：核心同步功能被限制次数，强制订阅付费。
- **场景割裂**：网页端、客户端、开发工具（如 VSCode）之间无法互通，打断开发者心流。
- **黑盒风险**：闭源工具存在隐私泄露风险，且粗暴的"全自动发布"极易触发平台风控导致封号。
- **国内环境适配差**：国内平台接口封闭、外链排斥、富文本与图片防盗链机制复杂，普通工具难以完美兼容。

### 1.2 项目愿景

为开发者、极客与内容创作者，提供最纯粹、最高效、最安全的跨平台分发体验。

- **Local First**：数据与鉴权完全在本地流转，绝对不碰用户密码。
- **Geek Driven**：深度融入开发工作流（VSCode / 浏览器双端齐发）。
- **Rule Compliant**：敬畏国内平台生态，以"安全草稿同步"为核心。
- **Open Source**：100% 开源，建立社区共建的适配器生态。

---

## ⚔️ 二、核心战略

### 2.1 形态战略：一核双翼，跨端协同

- **核心层（Core）**：纯 TypeScript 实现，剥离宿主环境依赖，负责 Markdown 解析、平台适配调度。
- **桌面翼（VSCode 插件）**：主攻开发者场景，利用 Node.js 原生读写本地文件，提供沉浸式编辑与发布体验。
- **网页翼（Chrome 扩展）**：主攻自媒体场景，利用 Manifest V3 获取网页登录态，支持图文混排抓取。

### 2.2 发布战略：国内草稿优先，海外 API 直发

- **国内平台**：坚决执行"一键全网建草稿"。同步完成后引导用户人工确认发布，彻底规避账号风控风险。
- **海外平台**：依托官方 API（Dev.to, Hashnode, Medium），支持直接发布。同时内置 AI 出海翻译（BYOK），一键生成英文长文。

### 2.3 迭代战略：首发核心盘，众包拓疆土

- **首发 MVP**：死磕技术核心社区"8 大金刚"。
- **后续扩展**：提供标准化适配器接口，通过开源社区众包，补齐泛媒体（小红书、头条、百家号）与云厂商社区。

---

## 🏗️ 三、技术架构与规范

### 3.1 Monorepo 目录结构（基于 pnpm workspaces）

```text
openpub/
├── packages/
│   ├── core/                 # 纯逻辑：Markdown解析、Adapter接口定义、图片处理
│   ├── ui/                   # 共用的 Webview UI 组件 (React / Vue + Tailwind)
│   └── adapters/             # 各大平台的适配器实现 (独立目录，方便社区PR)
├── apps/
│   ├── chrome-extension/     # Chrome 壳 (Manifest V3，兼容降级 V2)
│   └── vscode-extension/     # VSCode 壳 (Webview + Node 环境)
├── docs/                     # 开发文档与适配器编写指南
└── package.json
```

### 3.2 核心技术栈

| 类别     | 选例                                                              |
| -------- | ----------------------------------------------------------------- |
| 语言     | TypeScript（严控类型，保障跨端稳定性）                            |
| 构建工具 | Vite + @crxjs/vite-plugin（Chrome 端）/ vsce（VSCode 端）         |
| 解析引擎 | remark / turndown（Markdown 与 HTML 互相转换，处理代码高亮）      |
| UI 框架  | React 或 Vue3 + Tailwind CSS（打造现代极客风，暗黑模式优先）      |

### 3.3 核心接口规范：PlatformAdapter

所有平台适配器必须实现此接口，以保证架构的可维护性与社区扩展性。

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

## 🎯 四、平台路线图 (Roadmap)

### 🟢 V1.0.0 首发版（8 大金刚）

优先打通技术开发者基本盘，验证核心链路。

| 平台       | 主攻模式         | 核心难点 / 预案                                                              |
| ---------- | ---------------- | ----------------------------------------------------------------------------- |
| 微信公众号 | API + 浏览器     | `cgi-bin/draft/add` 接口，`<section>` 标签富文本转换，图片防盗链。            |
| 知乎       | API（内部接口）  | Cookie `z_c0` 鉴权，图片上传，防范营销外链判定。                              |
| CSDN       | API + 模拟       | 处理特定请求签名与 Tags 映射。                                                |
| 掘金       | API              | `POST /api/v2/drafts`，处理 Tags 的 ID 映射。                                 |
| 博客园     | API（MetaWeblog）| 古老但稳定的 XML-RPC 协议，纯服务端推送。                                     |
| 思否       | 模拟注入         | 风控严，采用 Content Script 模拟编辑器输入。                                  |
| 腾讯云     | 模拟注入         | 模拟用户事件，确保草稿正确保存。                                              |
| 阿里云     | 模拟注入         | 重点适配图片上传至阿里云 OSS 的拦截逻辑。                                     |

### 🟡 V1.1.0 拓荒版（泛媒体与海外）

- **国内泛媒体**：百家号、头条、小红书（长文/图文）、微博。
- **海外平台**：Dev.to, Hashnode, Medium。内置大模型（BYOK）实现中英双语一键出海。

### 🔵 V1.2.0+ 生态版

引入社区众包机制，开放 `CONTRIBUTING.md`，设立"悬赏榜"鼓励开发者提交 PR。

---

## 💰 五、开源生态与商业化 (Open Core)

坚决抵制破坏体验的弹窗广告与强制打赏。采用业界成熟的**开放核心 (Open Core)** 模式：

- **社区版 (Free)**：永久免费。包含 8 大核心平台同步、基础 Markdown 解析、有限次数的 AI 出海翻译。不嵌任何广告。
- **专业版 (Pro)**：按年/月订阅（如 ¥19/月）。包含：无限 AI 翻译（用户自带 API Key）、多账号矩阵管理、同步失败自动重试、完整同步历史记录、高级图片处理。
- **企业版 (Enterprise)**：定制化私有部署，团队许可证统一管理。

**收款方式**：通过外部独立站（Stripe / 聚合支付）完成收款，发放 License Key，插件内验证解锁。

---

## 🛡️ 六、免责声明与红线（必须置于 README 醒目位置）

- **数据隐私**：本插件完全在本地运行，所有平台 Cookie、Token 及文章数据绝不经过第三方服务器中转，代码开源可查。
- **网络免责**：本项目不提供任何形式的网络代理（翻墙）服务。海外平台与 AI 接口的连通性，依赖于用户本地的网络环境及用户自行配置的代理节点（支持 Local Proxy 或 Cloudflare Workers 自建）。
- **发布控制**：国内平台默认仅提交至"草稿箱"，最终发布权交由用户。因用户直接发布或频繁操作引发的平台限制/封号，本项目不承担相关责任。

---

## 👨‍💻 七、协作与贡献指南

- **开发规范**：遵循 Conventional Commits 提交规范（如 `feat: 新增掘金适配器`）。
- **CI/CD**：Github Actions 自动构建 `.crx` 和 `.vsix` 产物。
- **适配器开发**：提供 `create-adapter` 脚手架，开发者可一键生成模板，专注编写 `sync` 逻辑。