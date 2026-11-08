# Design

## Context

动机见 `proposal.md` 的 Why。以下是影响方案选择的现状事实：

- `App.tsx` 是单文件约 800 行，包含文章编辑区、平台选择、配置弹窗、日志区与同步流程；所有弹窗以 `{condition && (...)}` 内联渲染，无路由库、无状态管理库、无 Toast 基础设施。
- 凭据已加密落盘：`lib.rs` 有 `dpapi_encrypt` / `dpapi_decrypt`（Windows 走 DPAPI，非 Windows 返回不支持），`encrypt_and_store_cookie` 把 `{platform}_cookie` 写入 `credentials.json`，`load_cookie` / `clear_cookie` 已就绪。
- 配置与业务数据都在 `platform_config.json`：`{platform}_config`、`articles:index`、`articles:{id}`、`syncmap:{id}`。
- `auth-demo` 主 spec 已描述完整的 Webview 登录 + DPAPI + 事件回调契约，但实现只有 `App.tsx` 里的 `handleOpenWebview`：用前端 `new WebviewWindow(...)` 打开窗口，窗口关闭即结束，从未调用 Cookie API，也从未发出任何事件。这是 spec 与实现的真实落差。
- `PLATFORM_REGISTRY` 已含 `loginUrl`，`credentialKind` 区分 `metaweblog` / `cookie` / `webview`，`available` 与 `ADAPTER_IMPLEMENTED_IDS` 由 `auditRegistry()` 双向校验。
- Tauri 插件现有 `opener` 与 `store`；`capabilities/default.json` 只声明了 `core:default`、`opener:default`、`store:default`，且 `windows: ["main"]`。
- `reqwest` 已是 Rust 依赖，`windows-sys` 已启用 `Win32_Foundation`、`Win32_Security_Cryptography`、`Win32_System_Memory` 三个 feature。
- `public/` 下只有 `tauri.svg` 与 `vite.svg`，尚无打赏图。

## Goals / Non-Goals

**Goals:**

- 让新用户在不打开浏览器开发者工具的前提下完成任一 Cookie 类平台的鉴权
- 让配置可备份、可迁移、可彻底清除，且任何破坏性操作都有确认与回滚
- 让应用具备可被分享的门面：开源入口、赞助渠道、推荐位与运行环境信息
- 让版本升级有明确的人工确认路径，为后续 `tauri-plugin-updater` 接入留好接口
- 新增功能不破坏既有同步链路：`platform_publish` 命令签名与调用参数保持不变

**Non-Goals:**

- 不做静默自动更新、不做增量差分下载、不配置 updater 的公钥与签名清单
- 不做多账号、不做凭据跨机器加密迁移（导出为明文，见 Decision 5）
- 不引入路由库或状态管理库，不改动文章编辑区的分栏结构
- 不重构 `App.tsx` 中既有的平台选择、日志与同步流程
- 不实现微信公众号 / 思否 / 腾讯云 / 阿里云的登录与适配

## Decisions

### Decision 1: 作为独立变更 phase3，而不是扩展 phase2

新建 `phase3-product-experience`，不并入 `phase2-csdn-zhihu`。

**理由**：phase2 的剩余任务（CSDN / 知乎适配器）被浏览器抓包阻塞，契约未定；本变更的四个模块与适配器契约零重叠。合并后会让「等抓包」变成「阻塞所有产品化工作」，也会让归档时无法分辨哪些行为来自哪个意图。

**代价**：两者都会改 `App.tsx`。缓解方式是本变更只新增文件与新增 UI 挂载点，对 `App.tsx` 的改动集中在 header 与新增 overlay 两处，phase2 后续改动集中在平台提示与注册表字段，冲突面可控。

### Decision 2: 登录窗口由 Rust 侧创建，而不是前端

前端不再 `new WebviewWindow(...)`，改为调用 `open_login_window(platform)` 命令；Rust 用 `WebviewWindowBuilder` 创建窗口并挂 `on_page_load` 回调。

**理由**：Cookie 提取需要 `Webview` 句柄，且必须在页面加载完成后执行。前端创建窗口时拿不到 Rust 侧回调，只能靠轮询 `cookies()`，既浪费又容易在窗口销毁后拿到 `None`。由 Rust 创建窗口，可以在 `PageLoadEvent::Finished` 里直接取 `webview.cookies_for_url(url)`。

**备选**：前端创建 + Rust 侧定时轮询 Cookie。放弃原因是轮询无法区分「已登录」与「游客 Cookie 已写入」，会产生假成功。

窗口 label 采用 `login-{platform}`，Rust 侧维护 `HashMap<label, platform>` 映射以知道该窗口对应哪个平台。窗口关闭时清理映射。

### Decision 3: 登录成功判定用 Cookie 关键键，不用 URL 正则

判定「登录成功」的条件是：该平台 Cookie 中出现注册表声明的全部关键键，而不是匹配跳转后的 URL。

**理由**：URL 正则要为每个平台维护一条规则，且平台改版会静默失效；关键键判定直接复用适配器已有的鉴权前提（适配器本来就要校验 Cookie 含哪些键才能调用 API）。判定标准与鉴权标准同源，避免「登录判定通过但同步报未配置」的矛盾。

**配套**：`PlatformDefinition` 新增 `requiredCookieKeys: readonly string[]`。掘金为 `["sessionid"]`；知乎与 CSDN 先声明为空数组，关键键名称待 phase2 抓包确认（见 Open Question 1）。空数组意味着「任意非空 Cookie 视为已登录」，这是可接受的降级，且不引入未验证的猜测。

**注册表作为唯一事实来源**：登录 URL 与关键键都由前端从注册表读出后传入命令，即 `open_login_window(platform, loginUrl, requiredCookieKeys)`。Rust 侧不另建一份平台表，避免同一份数据在两处漂移；判定逻辑本身是纯函数，不依赖数据来源。

**防重**：每个登录窗口一个 `OnceFlag`，只允许一次落盘与一次成功事件。注意该标志守卫的是「最终落盘」而不是「每次 Cookie 读取」——首次页面加载发生在用户扫码之前，此时 Cookie 必然为空，若用标志拦住首次读取，后续就永远无法再提取。

**判定与提示的时机**：`PageLoadEvent::Finished` 时读取 Cookie 并判定。判定为「关键键齐全」才落盘、发 `login-success` 并关闭窗口；判定为「Cookie 为空」或「缺关键键」时不发事件、不关窗口，把判定结果暂存在会话里，等窗口被用户关闭时再发 `login-failed`（缺键时带上缺失键名）。这样首次页面加载不会误报失败，也不会打断用户扫码。

### Decision 4: 登录窗口使用独立 capability

新增 `capabilities/login.json`，`windows: ["login-*"]`，权限只含 `core:default` 与必要的 webview 读取权限，不含 `store:default`、`opener:default`、剪贴板。

**理由**：登录窗口加载的是第三方页面。Tauri 的 capability 按窗口 label 匹配，若沿用 `default.json`（`windows: ["main"]`）则登录窗口完全无权限、Cookie API 调用会被拒；若把 `login-*` 加进 `default.json`，第三方页面就获得了 store 读写与打开外链的能力。独立文件是唯一能同时满足「可读 Cookie」与「不越权」的做法。

### Decision 5: 导出为明文 JSON，但强制确认

导出文件包含明文 Cookie 与博客园 Token，不加密。

**理由**：备份的目的是跨机器迁移与人工可检查。DPAPI 密文绑定当前 Windows 用户与机器，导出的密文在另一台机器上无法解密，等于备份无效。若引入自定义口令加密，就要处理口令派生、口令丢失、错误口令提示三套逻辑，成本远高于收益，且用户仍需自己保管口令。

**缓解**：导出前弹出确认，文案明确「该文件包含明文 Cookie 与令牌，请妥善保管」；用户取消则不写文件。写入采用先写临时文件再重命名的方式，避免中途失败留下半个 JSON。

**明确拒绝的备选**：导出 DPAPI 密文（不可迁移）；导出时脱敏 Cookie（导入后仍需重新登录，失去备份意义）。

### Decision 6: 导入采用内存快照回滚，而非两阶段文件

导入流程：解析 JSON → 结构校验 → 读取当前 store 全部相关键为内存快照 → 清空并按新数据写入 → 保存。任一步失败则用快照重写并保存，恢复原状。

**理由**：`tauri-plugin-store` 的 `save()` 是整文件写入，本身不具备部分成功语义；真正的风险点是「清空成功但写入失败」，此时内存快照是唯一可靠的还原来源。双 store 文件方案（先写临时 store 再原子替换）需要绕过插件 API 直接操作文件，收益不匹配复杂度。

### Decision 7: 重置按键前缀精确清理，不做整文件清空

重置删除：`credentials.json` 中全部 `*_cookie` 键；`platform_config.json` 中 `articles:index`、全部 `articles:*`、全部 `syncmap:*`、以及注册表各平台的 `{id}_config`。

**理由**：整文件清空会把将来任何新增的非凭据键一起抹掉。按前缀与注册表驱动的键名清理，既覆盖当前全部凭据与业务数据，又为后续新增设置项留出安全边界。`localStorage` 中的界面偏好（已选平台、推广区关闭状态）不属于「数据」，明确不清除。

### Decision 8: 设置中心作为全屏 overlay，新 UI 拆到独立组件文件

设置中心是覆盖在主界面之上的全屏面板（`settingsOpen` 状态驱动），内部三个分区；新增 UI 拆到 `src/components/` 下的 `SettingsPanel.tsx`、`AboutDialog.tsx`、`Toast.tsx`，`App.tsx` 只保留挂载点与状态。

**理由**：`App.tsx` 已 800 行，再内联四个模块会突破可维护边界。但也不做整体重构——既有平台选择、日志、同步流程改动风险与本变更无关，把它们一起搬进组件会让 diff 无法审查。

**明确接受的技术债**：既有 Cookie 配置弹窗仍留在 `App.tsx` 内联。后续若要统一弹窗体系，应作为独立重构变更处理。

### Decision 9: Toast 单实例替换，不做队列

全局只有一个 Toast 状态，新消息覆盖旧消息，2.5 秒后自动消失。

**理由**：本变更的 Toast 只服务于「已复制到剪贴板」这类即时反馈，同一时刻不需要多条并存。队列会带来「连续点击复制后看到一串提示」的噪音。

### Decision 10: 版本检测走 Rust 命令 + 语义化比较，updater 仅留接口

新增 `check_update` 命令，用已有的 `reqwest` 拉取远端版本 JSON，按 `major.minor.patch` 逐段数值比较，返回 `{current, latest, hasUpdate, downloadUrl}`。前端只负责展示与打开链接。

**理由**：版本比较放在 Rust 侧可以用整数逐段比较，避免前端字符串比较把 `0.1.10` 判成小于 `0.1.9`。不启用 `tauri-plugin-updater` 的自动流程，是因为自动更新需要签名密钥、更新清单与 CI 产物签名，当前 Gitee 发布链路尚未具备；强行接入会得到一个「能检测但装不上」的半成品。

**版本比较规则**：缺失段视为 0，非数字段视为比较失败（返回检查失败而非判为最新）。这条规则专门防止「响应格式变了但界面显示已是最新」的静默错误。

**更新源常量**：远端版本 JSON 地址集中定义在 Rust 侧一处常量：

```
https://gitee.com/tianyeHome/OpenPub/raw/master/latest.json
```

响应格式为 `{"version": "0.1.0", "downloadUrl": "..."}`。用户稍后会在 Gitee 仓库根目录创建该文件；在其存在之前，请求失败按「未配置更新源」提示，不阻塞其他功能。所有对更新源的引用只允许通过这个常量，禁止在其他位置重复写字面量，以便后续一行替换。

### Decision 11: 运行环境信息从宿主读取，不引入 sysinfo

`get_host_info` 命令返回 `{appVersion, osVersion, engine}`。`appVersion` 取 `package_info().version`；`osVersion` 在 Windows 上走 `RtlGetVersion`（需为 `windows-sys` 增加 `Win32_System_SystemInformation` feature），其他平台回退为 `std::env::consts::OS` + `ARCH`；`engine` 固定为 `"Tauri v2 + Rust"`。

**理由**：`RtlGetVersion` 是 Windows 上唯一不撒谎的系统版本来源（`GetVersionEx` 会受兼容性清单影响而返回伪版本）；为了一行系统版本引入 `sysinfo`（一个带进程枚举能力的重依赖）不划算。

## Risks / Trade-offs

| 风险 | 影响 | 缓解 |
|---|---|---|
| `on_page_load` 回调内调用 Cookie API 若与 webview 线程互锁 | 登录流程卡死 | Tauri 官方文档明确 `cookies()` 在 Windows 上于同步命令或事件处理器中调用会死锁（wry#583）。因此 Cookie 读取一律放到 `tauri::async_runtime::spawn_blocking` 的独立线程，事件处理器只做派发不阻塞；失败即发 `login-failed`，不重试 |
| 第三方登录页改版导致关键键变化 | 判定永远不成功 | 关键键集中在注册表一处，改一行即可；未匹配时 `login-failed` 明确报缺哪个键 |
| 明文导出被用户误发到公开渠道 | 凭据泄露 | 导出前强制确认 + 文件名含 `credentials` 提示 + README 说明 |
| 导入覆盖后用户后悔 | 数据丢失 | 导入前二次确认；导入失败回滚；建议用户先导出一次 |
| 全屏 overlay 与分栏面板的事件竞争（`Esc`、遮罩点击） | 弹窗叠加错乱 | 设置中心打开时屏蔽主界面快捷键；关于弹窗叠在设置中心之上，关闭顺序后进先出 |
| 新增 `login-*` capability 未覆盖到实际窗口 label | 登录窗口拿不到 Cookie | 联调时先验证 `login-juejin` 窗口的权限生效，再扩展其他平台 |
| 远端版本 JSON 端点尚不存在 | 检查更新恒失败 | 端点定义为常量并集中一处；失败文案区分「网络」「响应格式」，不伪装成已是最新 |

## Open Questions

1. **CSDN 与知乎的 Cookie 关键键名称（待抓包验证，非既定事实）**：CSDN 的 `UserToken`、知乎的 `z_c0` 与 `_xsrf` 均**待抓包验证**——目前没有任何一组真实抓包证明它们出现在实际 Cookie 中，因此注册表当前对这两个平台声明空数组（任意非空 Cookie 视为已登录）。phase2 抓包结果确认后，只需补一行注册表数据，无需改动判定逻辑。
2. **远端版本 JSON 的托管地址与字段名**：用户已确认为 `https://gitee.com/tianyeHome/OpenPub/raw/master/latest.json`，字段为 `version` 与 `downloadUrl`，但该文件尚未创建。实现以常量占位（见 Decision 10），请求失败时按「未配置更新源」提示；文件上线后无需改代码。
3. **打赏二维码图片**：用户已放置 `public/images/alipay_qr.png` 与 `public/images/wechat_qr.png`，组件按 `/images/alipay_qr.png`、`/images/wechat_qr.png` 引用。仍保留缺图占位分支（`onError` 时显示「二维码待补充」而非破图），以便换图或漏打包时不出现破图。