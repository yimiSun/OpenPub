# 快发布 · OpenPub

开源的多平台内容分发中枢。写一次 Markdown，一键同步到国内 18 个内容平台；支持草稿与发布两条路径、同步映射去重、配置导出导入。

基于 Tauri 2 + React + TypeScript 构建，Windows 桌面应用。

## 平台矩阵

v1.0.0 登记 18 个国内平台。**真实发布** 10 个，**仅草稿** 8 个（勾选发布时如实返回「该平台暂不支持直接发布，请在平台后台手动发布」，绝不静默降级为草稿）。

| 平台 | id | 通道 | 能力 | 真机验证 |
| --- | --- | --- | --- | --- |
| 博客园 | `cnblogs` | direct（MetaWeblog） | 真实发布 | 通过 |
| 掘金 | `juejin` | direct | 仅草稿 | 通过 |
| CSDN | `csdn` | direct | 真实发布 | 通过 |
| 微信公众号 | `wechat` | direct | 仅草稿（纯文字） | 通过 |
| 简书 | `jianshu` | webview | 仅草稿 | 通过 |
| 51CTO | `51cto` | direct | 真实发布 | 通过 |
| 开源中国 | `oschina` | direct | 真实发布 | 通过 |
| 腾讯云开发者社区 | `tencentcloud` | direct | 真实发布 | 通过 |
| 阿里云开发者社区 | `aliyun` | direct | 仅草稿 | 通过 |
| 新浪微博 | `weibo` | direct | 真实发布 | 通过 |
| 百家号 | `baijiahao` | direct | 仅草稿 | 通过 |
| 网易号 | `netease` | direct | 仅草稿 | 通过 |
| 哔哩哔哩 | `bilibili` | direct | 仅草稿 | 通过 |
| 今日头条 | `toutiao` | webview | 真实发布 | 通过 |
| 知乎 | `zhihu` | webview | 真实发布 | 通过 |
| 小红书（图文） | `xiaohongshu` | webview | 真实发布 | 通过 |
| 抖音 | `douyin` | webview | 仅草稿 | 通过 |
| 思否 | `segmentfault` | webview | 真实发布 | 待验证（暂无测试账号） |

抖音图文与小红书长文笔记不在 v1.0.0 范围。

## 两条执行通道

- **direct（直发）**：Rust 侧用 Cookie + 静态请求头直接调用平台接口。需要页面 token、响应头 CSRF 或身份标识的平台，在发业务请求前先做一次预检（`shared/page_token.rs` / `shared/csrf.rs`），预检失败归类 `not_configured` 且不发起业务请求。
- **webview（网页模拟）**：在平台自己的页面里执行平台自己的脚本，本应用只搬运文章数据与结果。用于依赖运行时签名的平台——签名由平台页面自行产生，本应用不计算、不缓存、不伪造。

### 签名红线

凡契约中出现 `x-zse-*` / `x-zst-*` / `_webmsxyw` / `X-s-common` / gateway `keyv`+`ivd` 加密串，或依赖页面脚本执行的平台，一律走 `webview`，**不做逆向**。

唯一例外是 CSDN：阿里云网关的 `x-ca-key` 与 HMAC-SHA256 密钥已取得明确授权，签名在 `platforms/csdn.rs` 内本地计算。CSDN 轮换密钥时本应用会静默失效（表现为 `401` 或 `X-Ca-Key is not exist`），恢复方式是重取密钥、只改 `SECRET_KEY` 常量——这是该路径的固有维护成本。

页面脚本的硬约束：不得内联任何签名算法或密钥常量，只做「取编辑器、填内容、点按钮、读结果」。

## 功能

- **一键同步**：多平台并行分发，单平台失败不影响其余平台
- **草稿 / 发布双路径**：草稿先落、发布后推；部分成功保留草稿标识，下次同步走更新而非重复建稿
- **同步映射**：按正文哈希去重，避免重复建稿
- **一键登录**：`direct` 平台通过内嵌登录窗口扫码，凭据 DPAPI 加密存储
- **打开平台页面**：`webview` 平台拉起通道窗口就地登录，登录态由 WebView2 用户数据目录持有，不落 Cookie 凭据
- **设置中心**：配置导出 / 导入 / 重置；导出时 `webview` 平台数据不会被当作 Cookie 解密
- **版本检测**：启动检查新版本并引导下载

## 开发

### 环境要求

- Node.js ≥ 18、pnpm ≥ 9
- Rust stable（Tauri 2 工具链）
- Windows（WebView2 运行时，Win10/11 自带）

### 常用命令

```bash
pnpm install

# 桌面端开发（前端热更新 + Rust 重编译）
pnpm desktop:dev

# 构建安装包（nsis / msi）
pnpm desktop:build

# 全量构建（core 包 → 桌面应用）
pnpm build

# 代码检查
pnpm lint
pnpm typecheck
```

### 构建顺序（重要）

`packages/core` 的入口指向 `dist/`，源码改动不自动生效。修改 `packages/core/src` 下任何文件后，必须先生成 `dist` 再构建桌面应用，否则 TS 会拿旧输出报 `has no exported member`：

```bash
cd packages/core && pnpm build
cd apps/desktop-app && pnpm build
```

### 后端测试

```bash
cd apps/desktop-app/src-tauri
cargo test --lib
```

当前基线：406 个用例通过。

## 新增平台

三步，缺一不可：

1. **注册表**：`packages/core/src/platforms/registry.ts` 增加记录，声明 `channelType`（`direct` / `webview`）与 `available`（真机验证通过后才置 `true`），并把 id 加入 `ADAPTER_IMPLEMENTED_IDS`。
2. **登录白名单**（`direct` 平台）：`apps/desktop-app/src-tauri/src/login.rs` 的 `LOGIN_SUPPORTED_PLATFORMS`。
3. **凭据白名单**（`credentialKind: "cookie"` 平台）：`apps/desktop-app/src-tauri/src/settings.rs` 的 `COOKIE_CREDENTIAL_PLATFORMS`。

`webview` 通道平台**不进**上述两处白名单——其登录态由通道窗口自己持有，写进 Cookie 白名单会导致导出时解密崩溃。博客园走 MetaWeblog Token，同样不进 `COOKIE_CREDENTIAL_PLATFORMS`。

辅助脚本：`pnpm create-adapter` 生成适配器骨架。

## 目录结构

```
packages/core/              共享层：平台注册表、类型、同步映射
apps/desktop-app/
  src/                      React 前端
  src-tauri/src/
    platforms/              平台适配器（direct 通道）
    platforms/webview/      Webview 通道实现与页面脚本
    platforms/shared/       共享工具（cookie / csrf / page_token / markdown）
    login.rs                登录窗口
    settings.rs             设置中心与配置导出
    sync.rs                 同步编排
docs/                       项目总纲与参考实现
openspec/                   OpenSpec 变更制品
```

## 免责声明

本项目仅用于个人账号下的内容分发，不提供任何平台的签名逆向、风控绕过或批量账号运营能力。使用前请自行确认各平台的服务条款；因平台改版、密钥轮换或风控策略调整导致的失效，属于本项目的已知维护成本。

平台接口与页面结构随时可能变化，`webview` 通道的页面脚本尤其如此。失效时按平台逐个修复，不要把「昨天还能用」当作回归 bug。