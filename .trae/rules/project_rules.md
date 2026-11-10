# 项目约定

## 签名预判（接入新平台的第一道门槛）

接入任何新平台前，第一步必须是抓包预判——检查请求头是否含 `x-zse-*` / `x-ca-*` / `X-Signature` 等动态签名。**有签名 → 默认降级，不进实现阶段。**

- 判定顺序：① 有无官方开放 API（含正式密钥申请渠道）→ ② 是否纯 Cookie 无签名 → ③ 是否需要签名。
- 理由：CSDN（阿里云网关 `x-ca-*` HMAC-SHA256）与知乎（全站双层 `x-zse-96` + `x-zst-81`）各踩一次，均为签名阻塞，逆向成本不可接受。
- 默认不做逆向签名，也不做「先不带签名、失败再降级到签名」的混合方案——本质仍要维护易变密钥，只是延迟崩盘。
- 结论记入对应变更的 `design.md`，并在注册表保持 `available: false`、登录白名单不含该平台。

### 有签名 ≠ 不支持：改走 Webview 通道

v1.0.0 起，需要运行时签名的平台不降级为「不支持」，而是把执行位置换到平台自己的页面里（`adapterChannel: "webview"`）：应用只搬运文章数据与结果，签名由平台页面自行产生。真机验证通过后这些平台照样置 `available: true`。

- 判定口径：契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway `keyv`+`ivd` 加密串 / 依赖页面脚本执行的，一律 `webview`。
- 红线不变：页面脚本**不得**内联任何签名算法或密钥常量，只做「取编辑器、填内容、点按钮、读结果」。
- 通道契约见 `openspec/changes/v1.0.0-omni-release/specs/webview-injection-channel/spec.md`。

### 唯一例外：CSDN（已获明确授权）

CSDN 是本规则的**唯一例外**，已按明确授权恢复上线，与上面「默认降级」相反：

- 阿里云网关的 `x-ca-key`（`203803574`）与 HMAC-SHA256 密钥均已取得，签名在 `apps/desktop-app/src-tauri/src/platforms/csdn.rs` 内本地计算，注册表 `available: true`、登录白名单含 `"csdn"`。
- 记录见 `openspec/changes/v1.0.0-omni-release/design.md` 的 Decision 10。
- **风险已知并接受**：密钥硬编码在 CSDN 前端 JS 里，CSDN 换密钥时本应用会静默失效（表现为 `401` 或 `X-Ca-Key is not exist`），需重新取一次密钥并只改 `csdn.rs` 的 `SECRET_KEY` 常量。这不是回归 bug，是这条路径的固有维护成本。
- **例外范围仅限 CSDN 一家**。知乎的双层签名（其中一层疑似 WASM 产出）不在例外内，其余新平台仍按上面的默认规则判定；不得因为 CSDN 特例而放宽其他平台的签名门槛。

## 构建顺序

修改 `packages/core/src` 下的任何文件后，必须先执行 `pnpm build`（在 `packages/core` 目录）重新编译该包，再构建 `apps/desktop-app`。否则 TS 会拿 `@openpub/core` 的旧 `dist` 输出报 `has no exported member` 错误。

- `packages/core` 的入口指向 `dist/`（见 `package.json` 的 `main`/`types`），源码改动不自动生效。
- 命令顺序：`cd packages/core && pnpm build` → `cd apps/desktop-app && pnpm build`。

## 新平台接入双清单

接入新平台时，除前端注册表 `packages/core/src/platforms/registry.ts` 外，必须同步更新两处 Rust 侧名单，否则会出错：

1. `apps/desktop-app/src-tauri/src/login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`：
   - 枚举允许一键登录并写入凭据的平台（当前 `["juejin"]`）。漏加会导致 `open_login_window` 与 `persist_cookie` 拒绝该平台登录。
2. `apps/desktop-app/src-tauri/src/settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS`：
   - 枚举 `credentialKind: "cookie"` 的平台。漏加会导致导出把凭据错当 Cookie 解密而崩溃（博客园例外，走 MetaWeblog Token 不应列入）。

`persist_cookie` 有平台 allowlist，新增平台必须同时更新上述两处。

### `adapterChannel: "webview"` 的平台不进两处名单

`webview` 通道平台的登录态由通道窗口（`channel-{platform}`）自己持有，不写 DPAPI Cookie，因此：

- 不得列入 `LOGIN_SUPPORTED_PLATFORMS`（否则会用一个注定拿不到可用凭据的登录窗口抓 Cookie）。
- 不得列入 `COOKIE_CREDENTIAL_PLATFORMS`（否则导出时把这些平台的数据当 Cookie 解密会崩）。
- 前端凭据入口按 `adapterChannel` 渲染：`webview` 平台展示「打开平台页面」，而不是「一键登录」。

### credentialKind 必须与真实鉴权路径一致

`registry.ts` 中平台的 `credentialKind` 决定前端走哪套配置弹窗，也决定是否进入 `COOKIE_CREDENTIAL_PLATFORMS`：

- `credentialKind: "cookie"` → 前端走 Cookie 配置弹窗，且必须列入 `COOKIE_CREDENTIAL_PLATFORMS`。
- `credentialKind: "webview"` → 前端走内嵌登录窗口，凭据不参与 DPAPI Cookie 解密。
- `credentialKind: "metaweblog"` → 走 MetaWeblog Token（如博客园），**不得**列入 `COOKIE_CREDENTIAL_PLATFORMS`。

典型坑：微信公众号当前登记为 `"webview"`。若预判结论是「无签名、走后台 Cookie 路径」，**必须把 `credentialKind` 改为 `"cookie"`**，否则前端不会出现 Cookie 配置弹窗，也会漏掉 `COOKIE_CREDENTIAL_PLATFORMS` 双清单，导致凭据存了却发不出去。