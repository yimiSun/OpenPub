# Proposal

## Why

v1.0.0 商店版真机回归暴露出三类 P0 阻断问题，均属「实现细节与平台真实契约不符」，而非新功能缺失：

1. **富文本格式全平台丢失**。共享转换器 `shared/markdown.rs` 的 `to_html_content` 只做「转义 HTML 特殊字符 + 每行包 `<p>`」，代码块、列表、表格、引用全部退化成纯文本。10 个依赖该转换器的平台（微博、微信、知乎、头条、简书、网易号、B站、百家号、掘金、CSDN）在编辑器里表现为「挤成一坨」。
2. **抖音发布必失败（`status_code=8`）**。`ensure_login_window` / `watch_login` 在页面刚落到 `creator-micro` 路由的瞬间即判定登录成功并关窗，用户扫码尚未完成；随后 `run` 阶段带着空 Session 打 `draft` 接口，被抖音判为未登录。
3. **51CTO 标签报错「单个标签最大不能超过20字」**。`fifty_one_cto.rs` 把 `tags.join(",")` 的逗号替换成空格后整串提交，导致 51CTO 按空格切分得到的「单个标签」是全部标签拼成的长串，必然超限。

## What Changes

- **转换器升级**：`shared/markdown.rs` 的 `to_html_content` 改用 `comrak`（GFM：表格、删除线、自动链接、任务列表），并保留「空输入回落 `<p></p>`」与「单换行即换行」两条既有语义。
- **微信 HTML 层清洗**：`wechat.rs` 的图片占位与危险标签剔除从 Markdown 层搬到 HTML 层，`<img>` 一律替换为 `[图片]`，`<a>` 收敛为纯文本（公众号正文不支持可点外链）。
- **51CTO 标签归一化 + 正文转 HTML**：删除 `replace(',', " ")`，按 `[,，、]` 拆分、去空去重、逐标签截断 20 字符、内部空格转 `-`，拼接后超 100 字符则从尾部丢标签并提示；正文改传 `to_html_content`。
- **博客园正文转 HTML**：MetaWeblog `description` 改为 `xml_escape(to_html_content(...))`。
- **小红书正文纯文本化**：图文笔记正文传参前剥离 Markdown 符号，避免 `#`、`*` 直接显示在笔记描述里。
- **抖音正文转 HTML + 登录竞态修复**：`long_article` 改传 `to_html_content` 并打印前 100 字符；`watch_login` 改为「连续 3 次（1.5s）稳定命中才判定成功」；`status_code=8` 改走 `AUTH` 通道，Rust 侧显窗等待用户重新登录后自动重试一次。

## Capabilities

### Modified Capabilities

- `platform-shared-utils`: 「markdown to html conversion」由「逐行包 `<p>`」改为「GFM 解析」，并新增「空输入兜底」「单换行即换行」「平台清洗作用于 HTML」三条约束。
- `platform-adapters`: 新增 51CTO 标签归一化、抖音登录稳定性与鉴权失败重试、以及 51CTO / 博客园 / 小红书 / 抖音四处正文格式契约。

## Impact

- 后端：`src-tauri/Cargo.toml`、`platforms/shared/markdown.rs`、`platforms/wechat.rs`、`platforms/fifty_one_cto.rs`、`platforms/cnblogs.rs`、`platforms/webview/xiaohongshu.rs`、`platforms/webview/douyin.rs`、`platforms/juejin.rs`（仅测试断言）。
- 不受影响：`tencentcloud.rs`（`<!--markdown-->` 包裹是刻意契约）、`aliyun.rs`、`oschina.rs`（原生吃 Markdown）。
- 前端无改动。

---

## v1.0.6 合并：关于窗口 Windows 版本识别 + 阿里云 payload 契约锁

### Why

v1.0.5 商店版回归暴露两个问题：

1. **「关于」窗口系统版本显示错误（真缺陷）**。Windows 11（Build 26300）上关于窗口底部显示「系统 Windows 10.0.26300」，既缺主版本号也没有版本号（如专业版）。根因：Windows 底层 API（`RtlGetVersion`）对 Windows 11 依然返回 `dwMajorVersion=10, dwMinorVersion=0`，只能靠 Build 号（>= 22000）判断；`lib.rs` 的 `os_version_string()` 直接把 Major/Minor 拼进字符串，且没有调用 `GetProductInfo` 取版本。
2. **阿里云草稿「保存成功但网页端打开为空」的修复指令经查证不成立（误报）**。已把当前 `aliyun.rs` 与仓库根目录的 v1.0.0 快照（`.aliyun_v100.txt`）逐行比对：v1.0.5 **从未改动** `aliyun.rs`；当前 payload 与 v1.0.0 逐字一致；修复指令中给出的 `markdown` / `categoryId` / `tagIds` 结构在 v1.0.0 里从未存在，照搬反而是回归。但该契约此前只有零散断言、没有「字段名整体不许漂移」的锁定，为防未来回归，补一条契约锁测试并把查证结论归档进 spec。

### What Changes

- **Rust 侧修正 Windows 版本识别**（`src-tauri/src/lib.rs`）：
  - `os_version_string()` 增加 Build 号映射：`dwBuildNumber >= 22000` 显示为 Windows 11，否则沿用 `RtlGetVersion` 报告的 Major；
  - 新增 `GetProductInfo` 调用把 `OS_PRODUCT_TYPE` 映射为中文版本名（专业版 / 家庭版 / 企业版 / 教育版 / 专业工作站版 / 旗舰版等），未知类型回退为不显示版本名；
  - 输出格式：有版本名时 `Windows 11 专业版 (Build 26300)`，无版本名时 `Windows 11 (Build 26300)`；`RtlGetVersion` 失败仍走原 `fallback_os_version()`；
  - 新增纯函数 `display_windows_major_for_build`（Build→主版本映射）与 `windows_edition_name`（产品类型→版本名映射），配单测钉住 22000 边界与各版本常量。
- **阿里云草稿 payload 契约锁**（`src-tauri/src/platforms/aliyun.rs` 仅测试）：新增单测断言 payload 字段名集合与 v1.0.0 编辑器契约完全一致，且 `markdown` / `categoryId` / `tagIds` 三键永远不允许出现（出现即回归信号）。

### Capabilities

- `support-about`（MODIFIED）：操作系统版本须正确反映 Windows 11（Build >= 22000）并尽可能带出版本名（如专业版），不再把底层 API 谎报的 `10.0` 直接展示。
- `platform-adapters`（ADDED Requirement）：新增「aliyun draft payload matches the editor schema」，锁定 `putDraft` 请求体字段名契约与 Markdown 原文传递。

### Impact

- 后端：`src-tauri/src/lib.rs`（版本识别 + 单测）、`src-tauri/src/platforms/aliyun.rs`（仅新增单测）。
- 不改动：`aliyun.rs` 的 `build_draft_payload` 与发布链路、前端 `AboutDialog.tsx`（Rust 侧一处修复即可，前端零改动）。
- 依赖：`windows-sys` 0.59 既有 feature（`Win32_System_SystemInformation`）已包含 `GetProductInfo` 与 `PRODUCT_*` 常量，无新增依赖。