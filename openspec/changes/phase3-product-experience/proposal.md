# Proposal

## Why

OpenPub 目前的形态是「能用」而不是「敢推荐」：新用户第一次打开后，必须自己按 F12 从浏览器里复制 Cookie 粘贴进来，这一步把绝大多数非极客用户挡在门外；应用没有设置中心，配置散落在弹窗里无法备份、迁移或彻底清除；也没有任何关于页、开源入口与打赏渠道，项目无法被看见和支持。要成为可交付给普通开发者的商业级桌面产品，这些缺口必须在功能扩张之前补齐。

## What Changes

- 新增**全局设置中心**：独立设置入口，承载配置备份/恢复（导出/导入本地 JSON）、红线级「一键清空 Cookie 与文章数据」重置。
- 新增**一键登录**：平台配置处提供「一键登录」按钮，弹出受控 `WebviewWindow`，用户在窗口内扫码/登录后由 Rust 侧拦截并提取该 Webview 的 Cookie，DPAPI 加密落盘，前端收到事件后关闭窗口并把状态更新为「已登录」。手动粘贴 Cookie 降级为备用入口，不再作为唯一路径。
- 新增**关于/支持弹窗**：暗黑风 Modal，含项目愿景、Gitee 开源入口、支付宝/微信打赏二维码位、可关闭的极客推荐推广区（智谱 GLM、阿里云服务器），以及只读运行环境信息（App 版本、操作系统版本、Tauri v2 + Rust 引擎）。
- 新增**版本检测**：设置页「检查更新」按钮请求远端版本 JSON 并比对当前版本，提示前往 Gitee 下载；`tauri-plugin-updater` 仅预留接入点，本次不做静默自动更新。
- 修改**既有 Webview 登录能力**：由「只打开窗口」升级为「打开 + 拦截 Cookie + 加密落盘 + 事件回调」的完整闭环。
- 修改**桌面壳宿主能力**：新增剪贴板权限、App/OS 版本信息命令、多实例 Webview 登录窗口的 capability 声明。

## Capabilities

### New Capabilities

- `app-settings`: 全局设置中心的入口、配置导出/导入（打包平台配置、Token、文章映射为单个 JSON）、以及清空 Cookie 与文章数据的重置能力，含导入前的格式校验与失败回滚。
- `support-about`: 关于/支持弹窗的展示内容与交互契约，包含开源入口、打赏二维码位、可持久关闭的推广区、剪贴板复制 Toast 反馈、以及从宿主读取的运行环境信息展示。
- `update-check`: 版本检测行为，包含远端版本信息获取、版本比对规则、以及「发现新版本 → 引导前往 Gitee」的交互结果。

### Modified Capabilities

- `auth-demo`: 现有 spec 已描述 Webview 登录与 DPAPI 存储，但实现只打开了窗口、从未拦截 Cookie。本次把该能力落实为真实闭环，并新增「按平台选择登录页」「Cookie 完整性校验」「登录成功后前端状态刷新」三条需求。
- `tauri-shell`: 新增剪贴板写权限、App 版本与操作系统版本读取命令、以及为登录窗口声明独立的 capability 权限。

## Impact

- 前端：`apps/desktop-app/src/App.tsx`（设置入口、一键登录按钮、关于弹窗、Toast）、新增设置/关于相关组件与 `public/alipay.png`、`public/wechat.png` 占位图。
- 后端：`apps/desktop-app/src-tauri/src/lib.rs`（新增配置导出/导入、数据重置、版本信息、Cookie 拦截命令）、新增 webview 登录窗口管理模块。
- 依赖：新增 `@tauri-apps/plugin-clipboard-manager` 与其 Rust 插件；`tauri-plugin-updater` 仅登记依赖与配置占位，不启用自动更新流程。
- 权限：`capabilities/default.json` 增加剪贴板权限，并为登录窗口新增独立 capability 文件。
- 数据：`platform_config.json`（平台配置、文章、同步映射）与 `credentials.json`（加密 Cookie）成为导出/导入与重置的作用对象。
- 与 phase2-csdn-zhihu 无代码冲突：phase2 改动平台适配器与映射写入语义，本变更只动设置、鉴权入口与展示层；但两者都会改 `App.tsx`，建议按 phase3 先行、phase2 后续 rebase 的顺序落地。