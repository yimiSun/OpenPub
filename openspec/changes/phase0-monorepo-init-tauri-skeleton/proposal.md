# Proposal

## Why

OpenPub 项目需要从零搭建 Monorepo 基础设施与 Tauri Windows 桌面 App 骨架，为后续 Core 层逻辑开发、平台适配器实现和端到端发布链路验证提供可运行的工程基础。没有这一层，所有业务逻辑（Markdown 解析、Adapter 调度、鉴权管理）都无从落地。Phase 0 是整个 V1.0 MVP 的地基，必须先于一切业务代码完成。

## What Changes

- 新增 pnpm Monorepo 工作空间，包含 `packages/core`、`packages/ui`、`apps/desktop-app` 三个子包
- 新增 `packages/core` 中的 TypeScript 类型契约：`PlatformAdapter` 接口及其关联类型（`PublishOptions`、`SyncResult`）
- 新增 `apps/desktop-app` 的 Tauri v2 桌面 App 骨架，集成 React + Tailwind CSS，打通 `pnpm tauri dev` 调试环境
- 新增 `packages/ui` 的 React 组件库骨架（共享 UI）
- 新增 `create-adapter` 脚手架脚本，支持一键生成平台适配器模板代码
- 新增根目录的 `pnpm-workspace.yaml`、共享 `tsconfig.json`、ESLint + Prettier 配置
- 新增 Tauri Webview 内嵌登录示例，跑通"点击登录 → 弹窗扫码 → 获取 Cookie → DPAPI 加密存储"最小闭环
- 新增 Windows 打包配置，产出 `.msi` 或 `.exe` 安装包

## Capabilities

### New Capabilities
- `monorepo-infra`: Monorepo 工作空间基础设施，包括 pnpm workspaces 配置、共享 TypeScript 配置、构建与代码规范工具链
- `core-types`: Core 层类型契约，定义 PlatformAdapter 接口及所有关联类型，作为平台适配器与调度器之间的编译时保障
- `tauri-shell`: Tauri v2 桌面 App 壳，提供 Webview 窗口宿主、Rust 桥接、系统凭据访问、本地文件读写、Windows 打包等宿主能力
- `shared-ui`: 共享 React UI 组件库，基于 Tailwind CSS，提供暗黑模式优先的极客风基础组件
- `adapter-scaffold`: 适配器脚手架工具，通过 `create-adapter` 脚本一键生成符合 PlatformAdapter 接口的适配器模板代码
- `auth-demo`: Tauri Webview 内嵌登录最小闭环示例，验证 Cookie 获取与 DPAPI 加密存储链路

### Modified Capabilities

（无既有能力需要修改）

## Impact

- 新增根目录及子包的 `package.json`、`tsconfig.json` 等配置文件
- 新增 `pnpm-workspace.yaml` 定义工作空间结构
- 新增 `apps/desktop-app/src-tauri/` 目录（Rust 侧）
- 新增 `apps/desktop-app/src/` 目录（React 前端）
- 新增 `.cargo/config.toml`（如需要）
- 新增 ESLint + Prettier 统一代码规范
- 依赖项：TypeScript 5.x、React 18.x、Tailwind CSS 3.x、Vite 5.x、`@tauri-apps/cli` 2.x、`@tauri-apps/api` 2.x、`tauri-plugin-store`、Rust stable toolchain
- 构建产物：Windows `.msi` 或 `.exe` 安装包