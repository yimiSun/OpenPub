# Tasks

## 1. 依赖、权限与宿主能力（两个优先模块的共用底座）

- [x] 1.1 在 `src-tauri/Cargo.toml` 与 `apps/desktop-app/package.json` 加入剪贴板插件（`tauri-plugin-clipboard-manager` 与 `@tauri-apps/plugin-clipboard-manager`），验证 `cargo build` 与 `pnpm build` 均通过且锁文件已更新
- [x] 1.2 在 `lib.rs` 注册剪贴板插件并在 `capabilities/default.json` 增加 `clipboard-manager:allow-write-text`，验证主窗口可写入剪贴板、且权限条目拼写被 Tauri 构建接受
- [x] 1.3 实现 `get_host_info` 命令返回 `appVersion` / `osVersion` / `engine`，Windows 走 `RtlGetVersion`（为 `windows-sys` 增加 `Win32_System_SystemInformation` feature），验证单测断言返回结构不含凭据或路径字段
- [x] 1.4 新增 `capabilities/login.json`（`windows: ["login-*"]`，仅最小权限），验证该文件被 Tauri 构建加载且未包含 `store`/`opener`/剪贴板权限

## 2. 关于与支持（优先实现）

- [x] 2.1 在 `src/components/Toast.tsx` 实现单实例 Toast（新消息覆盖旧消息、2.5 秒自动消失），验证连续触发两次只显示最后一条
- [x] 2.2 在 `src/components/AboutDialog.tsx` 实现暗黑风弹窗骨架：遮罩、右上角关闭、`Esc` 关闭、点击遮罩关闭，验证三条关闭路径均可关闭且打开时背景不可交互
- [x] 2.3 在弹窗内实现项目愿景文案与「打开 Gitee」按钮（调用 `openUrl` 打开 `https://gitee.com/tianyeHome/OpenPub`），验证点击后应用窗口不发生页面跳转
- [x] 2.4 实现支付宝/微信二维码并排展示，来源为 `public/images/alipay_qr.png` 与 `public/images/wechat_qr.png`，验证两张图片缺失时各自显示占位文案而非破图
- [x] 2.5 在 `public/images/` 放置支付宝与微信二维码图片，验证 `pnpm build` 后 `dist/images/` 内包含这两个资源
- [x] 2.6 实现「极客推荐」推广区：智谱 GLM（`https://www.bigmodel.cn/invite?icode=ejw1KvHtq349R9J2498tZEjPr3uHog9F4g5tjuOUqno%3D`）与阿里云（`https://www.aliyun.com/minisite/goods?userCode=4ls1opon`），每条含「复制链接」与「浏览器打开」，验证两个按钮使用完全相同的链接字符串
- [x] 2.7 实现「不再显示」复选框，勾选后立即隐藏推广区并用 `localStorage`（键 `openpub_hide_promo`）持久化，验证重启后推广区仍隐藏、取消勾选后恢复显示
- [x] 2.8 把「复制链接」接入剪贴板 API 并在成功后弹出「已复制到剪贴板」Toast，验证失败路径显示复制失败提示且不出现成功提示
- [x] 2.9 实现弹窗底部运行环境只读展示（App 版本、操作系统版本、`Tauri v2 + Rust`），验证三项内容来自 `get_host_info` 且无可编辑控件
- [x] 2.10 在 `App.tsx` 顶部导航栏加入「关于/支持」入口并挂载弹窗，验证入口可见、弹窗可开可关且不影响主界面分栏布局

## 3. 自动化 Cookie 获取（优先实现）

- [x] 3.1 在 `packages/core/src/platforms/registry.ts` 的 `PlatformDefinition` 新增 `requiredCookieKeys`，掘金填 `["sessionid"]`、其余平台填空数组（知乎 / CSDN 关键键待抓包验证，见 design.md Open Questions），验证 `pnpm build` 类型检查通过且 `auditRegistry()` 无新增告警
- [x] 3.2 在 `src-tauri/src` 新增登录窗口模块：`open_login_window(platform)` 用 `WebviewWindowBuilder` 创建 `login-{platform}` 窗口并维护 label→platform 映射，验证重复调用同一平台不会创建第二个窗口而是聚焦已有窗口
- [x] 3.3 在登录窗口挂 `on_page_load(Finished)` 回调，按注册表 `requiredCookieKeys` 判定登录是否完成，验证单测覆盖「关键键齐全判成功」「缺键判未完成」「Cookie 为空判取消」三种输入
- [x] 3.4 实现 Cookie 组装：把该窗口可见的全部 Cookie 序列化为 `name=value; name=value` 字符串，验证单测断言多 Cookie 顺序稳定且不含空片段
- [x] 3.5 提取成功后调用既有 `encrypt_and_store_cookie` 落盘并发 `login-success` 事件、随后关闭窗口，验证落盘失败时发出 `login-failed` 而非成功事件
- [x] 3.6 用 `OnceFlag` 保证同一登录窗口只提取一次，验证窗口在平台内多次跳转后仍只产生一次落盘与一次事件
- [x] 3.7 把 `App.tsx` 的 `handleOpenWebview` 改为调用 `open_login_window` 并监听 `login-success` / `login-failed`，验证收到成功后平台状态变为「已登录」且配置弹窗自动关闭
- [x] 3.8 把 Cookie 配置弹窗改为「一键登录」主入口 + 折叠的手动粘贴备用入口，验证两条路径保存的凭据都能让平台显示为已配置
- [x] 3.9 在 `lib.rs` 注册 `open_login_window` 命令并加入 `invoke_handler`，验证前端调用不再出现「命令不存在」错误
- [x] 3.10 真机联调掘金：点击一键登录、窗口内扫码登录、确认状态变为「已登录」且 `credentials.json` 中为密文，验证重启应用后该平台仍显示已配置

## 4. 设置中心与数据管理

- [x] 4.1 实现 `export_config` 命令：把 `{platform}_config`、`articles:index`、`articles:*`、`syncmap:*` 打包为含 `version`/`exportedAt`/`platforms`/`articles`/`syncMap` 的 JSON，先写临时文件再重命名，验证导出文件可被 `serde_json` 反序列化回同结构
- [x] 4.2 实现 `import_config` 命令：结构校验 → 内存快照 → 清空并写入 → 保存，任一步失败用快照还原，验证单测覆盖「合法文件导入成功」「缺字段被拒」「写入失败后数据回到导入前」
- [x] 4.3 实现 `reset_all_data` 命令：删除 `credentials.json` 全部 `*_cookie`、`platform_config.json` 的文章与映射键及各平台 `{id}_config`，验证单测断言重置后 `list_articles` 返回空且平台被判定未配置
- [x] 4.4 在 `src/components/SettingsPanel.tsx` 实现三个分区与分区切换，验证同一时刻只渲染一个分区
- [x] 4.5 接入「导出配置」按钮与「包含明文凭据」确认弹窗，验证取消确认时不产生任何文件写入
- [x] 4.6 接入「导入配置」按钮与导入前二次确认，验证导入成功后文章列表与平台配置状态刷新、导入失败时界面数据不变
- [x] 4.7 接入红线「一键清空所有 Cookie 与文章数据」并实现二次确认，验证清空后界面回到无文章无配置状态且 `localStorage` 偏好未被清除
- [x] 4.8 在 `App.tsx` 顶部导航栏加入「设置」入口并挂载全屏 overlay，验证设置中心打开时主界面快捷键被屏蔽、关闭后恢复

## 5. 版本升级检测

- [x] 5.1 实现语义化版本比较函数（缺失段补 0、非数字段判失败），验证 `0.1.0` vs `0.1.10` 判为有新版本、`1.0.0` vs `1.0.0` 判为最新、含非数字段判为失败
- [x] 5.2 实现 `check_update` 命令：请求远端版本 JSON、按上述规则比较、返回 `{current, latest, hasUpdate, downloadUrl}`，验证网络失败与响应格式异常分别返回可区分的原因
- [x] 5.3 在设置中心接入「检查更新」按钮与加载态，验证请求进行中按钮禁用且不可重复点击
- [x] 5.4 实现发现新版本的提示与「前往 Gitee 下载」按钮（走 `openUrl`），验证已是最新时不展示下载按钮
- [x] 5.5 在 `Cargo.toml` 与 `tauri.conf.json` 预留 `tauri-plugin-updater` 的依赖与配置注释占位（不启用自动流程），验证构建通过且应用启动时不发起任何自动更新请求

## 6. 集成验证

- [x] 6.1 运行 `cargo test`（`apps/desktop-app/src-tauri`），验证全部单测通过且既有平台适配器单测数量未减少
- [x] 6.2 运行 `pnpm build`，验证类型检查与生产构建通过
- [x] 6.3 冷启动检查：启动应用确认无白屏残留、`boot-splash` 正常淡出，验证首帧即为深色背景
- [x] 6.4 完整回归一次同步流程（博客园 + 掘金），验证一键登录保存的凭据可直接用于发布且日志阶段标注正确
- [x] 6.5 依次执行导出 → 重置 → 导入，验证导入后平台状态与文章映射与导出时一致

## Workflow follow-up

- 在实现与集成验证全部通过后归档该变更。
- 归档前把 Open Question 1（CSDN/知乎 Cookie 关键键）与 Open Question 2（远端版本 JSON 地址）的最终结论写回 `design.md` 与对应 spec。