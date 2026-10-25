# Design

## Context

OpenPub 是一个全新项目，当前目录下仅有 OpenSpec 配置和项目总纲文档，无任何代码。Phase 0 需要从零搭建完整的工程基础设施，为后续 Core 层业务逻辑和平台适配器开发提供可运行、可调试、可扩展的工程骨架。

关键约束：
- **首发仅做 Windows 桌面端 App，基于 Tauri v2**
- 放弃浏览器插件和 VSCode 插件形态（延至 V1.2+ 再考虑）
- 鉴权策略：Tauri Webview 内嵌平台登录页，通过 Tauri API 获取 Cookie，使用 DPAPI 加密落盘
- 所有代码纯 TypeScript 严格模式（Rust 侧仅做最小桥接）
- macOS 及苹果签名暂不处理，首发仅生成 Windows `.msi` / `.exe`

## Goals / Non-Goals

**Goals:**
- 建立可运行的 pnpm Monorepo 工作空间，子包间依赖正确解析
- 定义 Core 层类型契约，确立 PlatformAdapter 接口为所有适配器的编译时保障
- 搭建 Tauri v2 桌面 App 骨架，集成 React + Tailwind CSS，支持 `pnpm tauri dev` 热调试
- 提供共享 UI 组件库骨架，支持暗黑模式优先
- 提供 `create-adapter` 脚手架，一键生成适配器模板
- 跑通 Windows 打包链路，产出可安装的 `.msi` 或 `.exe`
- 跑通 Tauri Webview 内嵌登录示例，验证 Cookie 获取与加密存储链路

**Non-Goals:**
- 不实现任何平台适配器的业务逻辑（Phase 1 的工作）
- 不实现 Markdown 解析引擎（Phase 1 的工作）
- 不实现完整的鉴权流程与多账号管理（Phase 1 的工作）
- 不搭建 macOS 构建与签名流程（远期规划）
- 不实现 AI 翻译模块（V1.1 的工作）
- 不做 Chrome 扩展 / VSCode 插件（V1.2+ 远期规划）

## Decisions

### Decision 1: Tauri v2 over Electron / Flutter Desktop

**选择**：Tauri v2

**理由**：
- 安装包体积 2–10 MB，内存占用 30–40 MB，远优于 Electron（80–150 MB / 150–300 MB）
- 复用 React + TypeScript 技术栈，与 Core 层逻辑无缝共享
- 不捆绑 Chromium，直接调用系统 WebView，冷启动 <1 秒
- 内置系统凭据管理、文件系统、Webview 等 API，适配 PC App 需求

**备选**：Electron（体积和内存代价过大）、Flutter Desktop（需用 Dart 重写 UI，无法复用 Core 逻辑）

### Decision 2: pnpm workspaces over npm/yarn workspaces

**选择**：pnpm workspaces

**理由**：
- 严格依赖隔离，防止幽灵依赖，对 Monorepo 尤其重要
- 磁盘效率高，硬链接共享依赖
- `pnpm -r run` 原生支持拓扑排序执行

**备选**：npm workspaces（无幽灵依赖防护）、yarn workspaces（PnP 配置复杂度过高）

### Decision 3: React 18 + Tailwind CSS for Desktop UI

**选择**：React 18 + Tailwind CSS + shadcn/ui 风格自建组件

**理由**：
- 与 Core 层 TypeScript 类型系统无缝共享
- Tailwind 原子类在 Tauri WebView 中渲染稳定
- 自建组件避免依赖更新缓慢的第三方 UI 库
- 暗黑模式通过 `dark:` 变量原生支持

**备选**：Vue3（生态弱于 React）、Svelte（生态不足）、Ant Design（过重且风格不极客）

### Decision 4: Vite for UI builds, tsc for Core builds

**选择**：Vite 构建 React 前端，`tsc` 构建 Core 纯逻辑包

**理由**：
- Vite 对 React + Tailwind 的 HMR 支持极佳，`pnpm tauri dev` 调试体验好
- Core 是纯 TypeScript 库，无需打包，`tsc` 直接输出 `.d.ts` + `.js`
- 避免对纯逻辑包引入不必要的 bundler 依赖

**备选**：全 Vite（Core 不需要 HMR）、全 tsc（前端开发体验差）

### Decision 5: Tauri Webview 内嵌登录页鉴权

**选择**：Tauri 创建受控 Webview 窗口加载平台登录页，通过 Tauri API 获取 Cookie，DPAPI 加密后存入 `tauri-plugin-store`

**理由**：
- 用户扫码/登录体验最丝滑，无需手动复制 Cookie
- 登录发生在平台官方页面，用户凭据不经过 App 代码
- DPAPI 加密绑定当前系统用户，换机/换用户无法解密
- 首发版最简实现，无需跨端通信或本地服务

**备选**：手动填 Cookie（体验差）、浏览器插件桥接（引入额外进程）、本地 mini server（复杂度过高）

### Decision 6: create-adapter as Node.js script

**选择**：`scripts/create-adapter.js` 作为 Node.js 脚本

**理由**：
- 无需引入 Yeoman 等重框架
- 可直接操作文件系统和 `pnpm-workspace.yaml`
- 社区贡献者无需全局安装额外工具

**备选**：Yeoman generator（过重）、plop（额外依赖）

## Risks / Trade-offs

- **Tauri Rust 桥接代码 AI 生成幻觉率高** → 所有 Rust 侧代码必须逐行审查，禁止 `unwrap_or_default()` 静默吞错，必须用 `Result<T, E>` 传递错误到前端
- **Tauri v2 同步命令默认跑在主线程** → 耗时操作必须用 `async` 命令，避免 UI 卡死
- **Windows DPAPI 加密依赖系统用户** → 换用户登录后无法解密，需要在文档中明确告知，并提供"一键清除登录数据"入口
- **Tailwind CSS 在 Tauri WebView 中的 CSP 限制** → 构建时确保 Tailwind 产物内联或通过 Tauri 允许的本地路径加载
- **AI 生成的 React/Tailwind 代码存在"套话"问题** → 必须补齐 Loading / Empty / Error 三态，禁用紫色渐变等 AI 味设计，使用 ESLint + TypeScript 严格模式兜底
- **create-adapter 脚本修改 pnpm-workspace.yaml 可能引入格式问题** → 使用 `js-yaml` 读写，而非正则替换