# Tasks

## 1. Monorepo 根目录基础设施

- [x] 1.1 在项目根目录创建 `pnpm-workspace.yaml`，声明 `packages/core`、`packages/ui`、`packages/adapters/*`、`apps/desktop-app` 为 workspace 成员，并验证 `pnpm install` 无报错
- [x] 1.2 创建根 `package.json`，设置 `"private": true`，定义 `build`、`dev`、`lint`、`typecheck` 脚本（使用 `pnpm -r run`），并验证 `pnpm run build` 可执行（即使子包尚无构建产物）
- [x] 1.3 创建根 `tsconfig.json`，启用 `strict`、`esModuleInterop`、`skipLibCheck`、`resolveJsonModule`、`forceConsistentCasingInFileNames`，设置 `target: ES2022`、`moduleResolution: bundler`，并验证配置可被子包正确继承
- [x] 1.4 配置 ESLint（`eslint.config.js`）+ Prettier（`.prettierrc`），安装 `@typescript-eslint/parser`、`@typescript-eslint/eslint-plugin`、`eslint-config-prettier`，并验证 `pnpm run lint` 可执行
- [x] 1.5 创建 `.gitignore`，排除 `node_modules`、`dist`、`target`、`.tauri`，并验证 `git status` 不显示构建产物

## 2. packages/core 类型契约

- [x] 2.1 创建 `packages/core/package.json`，包名 `@openpub/core`，配置 `main`/`module`/`types` 指向 `dist/`，设置 `scripts.build` 为 `tsc -p tsconfig.json`，并验证 `pnpm install` 正确链接
- [x] 2.2 创建 `packages/core/tsconfig.json`，继承根配置，设置 `rootDir`/`outDir` 为 `src`/`dist`，并验证 `tsc --noEmit` 通过
- [x] 2.3 在 `packages/core/src/types.ts` 中定义 `PublishOptions` 接口（`title: string`、`markdown: string`、`tags?: string[]`、`coverImage?: string`、`isDraftOnly: boolean`），并验证 TypeScript 编译通过
- [x] 2.4 在 `packages/core/src/types.ts` 中定义 `SyncResult` 接口（`success: boolean`、`platform: string`、`draftUrl?: string`、`errorMsg?: string`），并验证 TypeScript 编译通过
- [x] 2.5 在 `packages/core/src/types.ts` 中定义 `PlatformAdapter` 接口（`readonly id: string`、`readonly name: string`、`readonly env: ("browser" | "api")[]`、`checkAuth(): Promise<boolean>`、`preprocess?(options: PublishOptions): Promise<PublishOptions>`、`uploadImages(images: string[], context: any): Promise<string[]>`、`sync(options: PublishOptions, context: any): Promise<SyncResult>`），并验证 TypeScript 编译通过
- [x] 2.6 创建 `packages/core/src/index.ts`，导出所有公共类型（`PublishOptions`、`SyncResult`、`PlatformAdapter`），并验证 `pnpm run build` 产出 `dist/index.js` + `dist/index.d.ts`

## 3. packages/ui 共享组件库骨架

- [x] 3.1 创建 `packages/ui/package.json`，包名 `@openpub/ui`，配置 Vite 库模式构建（`build.lib` 入口为 `src/index.ts`），安装 `react`、`react-dom`、`tailwindcss`、`autoprefixer`、`postcss`、`vite`、`@vitejs/plugin-react` 为依赖，并验证 `pnpm install` 通过
- [x] 3.2 创建 `packages/ui/tsconfig.json`，继承根配置，添加 `jsx: "react-jsx"`，并验证 `tsc --noEmit` 通过
- [x] 3.3 创建 `packages/ui/tailwind.config.js`，设置 `darkMode: "class"`，配置 `content` 扫描 `src/**/*.{ts,tsx}`，并验证 Tailwind 编译不报错
- [x] 3.4 创建 `packages/ui/postcss.config.js`，配置 `tailwindcss` + `autoprefixer` 插件
- [x] 3.5 在 `packages/ui/src/` 下创建基础组件：`Button.tsx`、`Input.tsx`、`Select.tsx`、`TagInput.tsx`、`StatusBadge.tsx`、`Card.tsx`，每个组件使用 Tailwind `dark:` 变量实现暗黑模式优先样式，并验证所有组件 TypeScript 编译通过
- [x] 3.6 创建 `packages/ui/src/index.ts`，导出所有组件，并验证 `pnpm run build` 产出库产物

## 4. apps/desktop-app Tauri 桌面 App 骨架

- [x] 4.1 使用 `create-tauri-app` 初始化 `apps/desktop-app`（React + TypeScript 模板），选择 Tauri v2，并验证 `pnpm install` 与 `pnpm tauri dev` 能启动默认窗口
- [x] 4.2 在 `apps/desktop-app/src-tauri/Cargo.toml` 中添加 `tauri-plugin-store` 依赖，在 `src-tauri/src/main.rs` 中注册插件，并验证编译通过
- [x] 4.3 在 `apps/desktop-app/src-tauri/src/lib.rs` 中实现基础命令 `greet`（返回示例字符串），声明为 `async`，返回 `Result<String, String>`，并验证前端可通过 `@tauri-apps/api/core` 的 `invoke` 调用
- [x] 4.4 在 `apps/desktop-app/src/` 中搭建三栏式基础 UI 布局（左侧：文章编辑区占位；右侧：平台勾选面板占位；底部：同步日志区占位），引入 `@openpub/ui` 组件和 Tailwind CSS，并验证 `pnpm tauri dev` 中 UI 正常渲染
- [x] 4.5 在 `apps/desktop-app/src-tauri/src/lib.rs` 中实现示例命令 `encrypt_and_store_cookie(platform: String, cookie: String)`，使用 Windows DPAPI（`windows-sys` 的 `CryptProtectData`）加密后以 Base64 存入 `tauri-plugin-store` 的 `credentials.json`，并验证存储文件中不含明文 Cookie
- [x] 4.6 在 `apps/desktop-app/src/` 中实现一个"测试登录"按钮，点击后打开 Tauri Webview 窗口加载 `https://juejin.cn/login`，验证 Webview 可打开且可扫码登录
- [x] 4.7 配置 `apps/desktop-app/src-tauri/tauri.conf.json` 中的 Windows 打包目标（`nsis` 或 `msi`），并验证 `pnpm tauri build` 能成功产出安装包
- [x] 4.8 在根 `package.json` 中添加 `"desktop:dev": "pnpm --filter @openpub/desktop-app tauri dev"` 和 `"desktop:build": "pnpm --filter @openpub/desktop-app tauri build"` 脚本，并验证可执行

## 5. create-adapter 脚手架脚本

- [x] 5.1 创建 `scripts/create-adapter.mjs`，接受平台名称参数，在 `packages/adapters/<name>/` 下生成 `package.json`（包名 `@openpub/adapter-<name>`，依赖 `@openpub/core`）、`tsconfig.json`（继承根配置）、`src/index.ts`（导出 adapter）、`src/adapter.ts`（实现 `PlatformAdapter` 接口的 stub），并验证生成的文件结构完整
- [x] 5.2 在 `scripts/create-adapter.mjs` 中实现 `pnpm-workspace.yaml` 自动更新逻辑（使用 `js-yaml` 解析/序列化），确保新适配器路径被加入 workspace，并验证 `pnpm install` 可解析新适配器包
- [x] 5.3 在根 `package.json` 中添加 `"create-adapter": "node scripts/create-adapter.mjs"` 脚本，并验证 `pnpm create-adapter test-platform` 可成功执行且生成的适配器 `tsc --noEmit` 通过
- [x] 5.4 清理测试产物：删除脚手架生成的 `test-platform` 测试目录，并验证 `pnpm install` 恢复正常（注：`packages/adapters/cnblogs/` 为真实适配器，保留并在 Phase 1 继续演进）

## 6. 端到端集成验证

- [x] 6.1 在项目根目录执行 `pnpm install && pnpm run build`，验证所有子包构建成功无报错
- [x] 6.2 执行 `pnpm run typecheck`，验证所有子包类型检查通过
- [x] 6.3 执行 `pnpm run lint`，验证 ESLint 无报错（允许 warning）
- [x] 6.4 执行 `pnpm desktop:dev`，验证 Tauri 桌面窗口启动、React UI 渲染正常、`greet` 命令可调用、Webview 登录弹窗可打开
- [x] 6.5 执行 `pnpm desktop:build`，验证 Windows 安装包成功产出，且可在 Windows 机器上安装并启动

## Workflow follow-up

- 在项目的审查要求满足后归档该变更。
- 验证归档后的结果。