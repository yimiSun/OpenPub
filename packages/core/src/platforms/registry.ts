export type CredentialKind = "metaweblog" | "cookie" | "webview";

/** 平台所属区域，决定侧边栏一级分组（国内平台 / 海外平台）。 */
export type PlatformRegion = "domestic" | "overseas";

/** 发布通道：直发（接口直发、无弹窗）/ 网页模拟（拉起窗口模拟点击）。 */
export type ChannelType = "direct" | "webview";

export interface PlatformDefinition {
  id: string;
  name: string;
  credentialKind: CredentialKind;
  loginUrl: string;
  requiredCookieKeys: readonly string[];
  /**
   * 除 Cookie 之外还必须填写的配置项名称（语义：`{platform}_config` 对象里这些键非空）。
   * 与 Cookie 无关，不要误以为它会检查 Cookie 内容。
   * 当前仅微信公众号需要 `token`（token 在后台 URL 里，不在 Cookie 里，扫码也拿不到）。
   */
  requiredConfigKeys?: readonly string[];
  /** 所属区域，决定侧边栏一级分组。 */
  region: PlatformRegion;
  /**
   * 发布通道，决定国内平台下的二级分组。
   * `webview` 平台的登录态由通道窗口自己持有，不写 DPAPI Cookie，
   * 因此 `credentialKind` 必须同为 `webview`，且不得列入 Rust 侧的
   * `COOKIE_CREDENTIAL_PLATFORMS`。
   */
  channelType: ChannelType;
  available: boolean;
  /**
   * 该 `webview` 平台是否拥有**独立扫码登录窗口**（走 `open_login_window`）。
   *
   * 为 `true` 时前端「打开平台页面」按钮直接拉起登录窗口，登录态由 WebView2
   * 的用户数据目录记住，与发布通道窗口共享 Cookie；为 `false`（默认）时
   * 平台只能在发布过程中被动登录。
   *
   * 配套要求：`requiredCookieKeys` 必须非空（否则窗口刚打开就被判定「已登录」），
   * 且 Rust 侧 `login.rs` 的 `WEBVIEW_LOGIN_PLATFORMS` 要同步登记。
   */
  loginWindow?: boolean;
  /**
   * 强制手动粘贴 Cookie，不提供网页扫码登录入口（兜底开关）。
   *
   * 为 `true` 时前端只展示 Cookie 输入框，不渲染「打开网页扫码登录」按钮。
   * 仅在登录窗口确实拿不到可用 Cookie 时才使用；正常情况下应保持缺省（false），
   * 由前端同时提供「扫码登录」主入口与「手动粘贴」折叠备用入口。
   *
   * 注意：`requiredCookieKeys` 为空并不等于只能手动——Rust 侧登录窗口会改走
   * 「URL 跳转 / DOM 探针确认登录态后抓取全量 Cookie」，因此这类平台同样
   * 支持扫码登录，不得再标记为 `manualCookieOnly`。
   */
  manualCookieOnly?: boolean;
}

export const PLATFORM_REGISTRY: readonly PlatformDefinition[] = [
  {
    id: "cnblogs",
    name: "博客园",
    credentialKind: "metaweblog",
    loginUrl: "https://i.cnblogs.com/settings",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "juejin",
    name: "掘金",
    credentialKind: "cookie",
    loginUrl: "https://juejin.cn/",
    requiredCookieKeys: ["sessionid"],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "zhihu",
    name: "知乎",
    credentialKind: "webview",
    loginUrl: "https://www.zhihu.com/signin",
    // `z_c0` 是知乎真正的登录态 Cookie：未登录时首页只会下发 `_xsrf` / `d_c0` 等匿名
    // Cookie，只有扫码成功后才出现 `z_c0`。用它当判据，登录窗口才不会刚打开就被
    // 误判为「已登录」并自动关闭。
    requiredCookieKeys: ["z_c0"],
    region: "domestic",
    channelType: "webview",
    loginWindow: true,
    available: true,
  },
  {
    id: "csdn",
    name: "CSDN",
    credentialKind: "cookie",
    loginUrl: "https://passport.csdn.net/login",
    requiredCookieKeys: ["UserToken"],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "jianshu",
    name: "简书",
    // 简书的接口按客户端 TLS/HTTP 指纹做风控（连不带 Cookie 的公开首页都返回
    // `403 code 9 异常请求`），Rust 侧无论怎么补 Origin / Referer / Sec-Fetch-*
    // 都无法通过。发布与登录因此改到 `channel-jianshu` 通道窗口的页面上下文里
    // 执行，登录态由通道窗口自己持有，不写 DPAPI Cookie。
    credentialKind: "webview",
    loginUrl: "https://www.jianshu.com/sign_in",
    // 登录判据是「URL 从 `/sign_in` 跳回站内」（见 `webview/jianshu.rs`），
    // 不依赖具体 Cookie 键；`loginWindow` 平台不会把该字段传给 `open_login_window`。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    loginWindow: true,
    available: true,
  },
  {
    id: "wechat",
    name: "微信公众号",
    credentialKind: "cookie",
    loginUrl: "https://mp.weixin.qq.com/",
    requiredCookieKeys: [],
    requiredConfigKeys: ["token"],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "segmentfault",
    name: "思否",
    credentialKind: "webview",
    loginUrl: "https://segmentfault.com/",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    // 思否没有独立的登录弹窗，但 `channel-segmentfault` 通道窗口本身就是写作页
    // （`https://segmentfault.com/write`），未登录访问会被重定向到 `/signin`。
    // 因此「配置」按钮直接把通道窗口拉到前台让用户就地登录即可，不需要弹
    // 「发布时会自动打开平台页面」那种兜底提示（见 `webview/segmentfault.rs`
    // 的 `open_login_channel`）。
    loginWindow: true,
    available: true,
  },
  {
    id: "tencentcloud",
    name: "腾讯云",
    credentialKind: "cookie",
    loginUrl: "https://cloud.tencent.com/login",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "aliyun",
    name: "阿里云",
    credentialKind: "cookie",
    loginUrl: "https://account.aliyun.com/login/login.htm",
    // `c_csrf` 是匿名下发的防 CSRF 令牌，未登录也存在，不能当登录态判据。
    // 因此留空，登录态改由 Rust 侧 Cookie 轮询确认（`COOKIE_POLL_PLATFORMS`
    // 含 aliyun，专属判据 `aliyun_logged_in_cookie` 只认 HttpOnly 的
    // `login_aliyunid_ticket` / `login_aliyunid`）。不得用 URL 或 DOM 探针判定。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "51cto",
    name: "51CTO",
    credentialKind: "cookie",
    // 旧的 `passport.51cto.com` 已下线，DNS 直接解析不到（ERR_NAME_NOT_RESOLVED）。
    // 现统一从博客主站的登录入口进入，登录成功后由主站自己跳转到后台。
    loginUrl: "https://blog.51cto.com/login",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "oschina",
    name: "开源中国",
    credentialKind: "cookie",
    loginUrl: "https://www.oschina.net/home/login",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "baijiahao",
    name: "百家号",
    credentialKind: "cookie",
    loginUrl: "https://baijiahao.baidu.com/",
    // 登录票据 `BDUSS` 由百度通行证下发且是 HttpOnly，页面内读不到，因此
    // `requiredCookieKeys` 留空，登录态改由 Rust 侧 Cookie 轮询确认
    // （`COOKIE_POLL_PLATFORMS` 含 baijiahao，专属判据
    // `baijiahao_logged_in_cookie` 只认 `BDUSS` / `BDUSS_BFESS` /
    // `STOKEN` / `PTOKEN`）。不得用 URL 或 DOM 探针判定。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "netease",
    name: "网易号",
    credentialKind: "cookie",
    loginUrl: "https://mp.163.com/",
    // 登录票据 `NTES_YD_SESS`（网易号业务域 mp.163.com 扫码后下发）是 HttpOnly，
    // 页面内读不到，因此 `requiredCookieKeys` 留空，登录态改由 Rust 侧 Cookie 轮询
    // 确认（`COOKIE_POLL_PLATFORMS` 含 netease，专属判据
    // `netease_logged_in_cookie` 主认 `NTES_YD_SESS`，兜底 `NTES_SESS` / `NTES_SESSID`，
    // 后两者是网易通行证主站 passport.163.com 的键名，不要当成业务域票据）。
    // 不得用 URL 或 DOM 探针判定。`P_INFO` / `S_INFO` 等匿名即带值的埋点字段不可作判据。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "bilibili",
    name: "哔哩哔哩",
    credentialKind: "cookie",
    loginUrl: "https://passport.bilibili.com/login",
    requiredCookieKeys: ["bili_jct"],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "weibo",
    name: "新浪微博",
    credentialKind: "cookie",
    loginUrl: "https://weibo.com/login.php",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
    available: true,
  },
  {
    id: "toutiao",
    name: "今日头条",
    // 头条发布接口带页面级风控：Rust 侧把原始插件 `buildArticleBody` 的 27 个字段
    // 拼得一模一样，`reqwest` 直发仍稳定返回 `7050 参数校验失败`。原始插件
    // `index.mjs` 的 `publishViaEditor` 是把 `fetch` 注入到头条编辑页的页面上下文里
    // 执行，浏览器环境特征与页面 SDK 自动带上，才不会被判伪造。因此发布改到
    // `channel-toutiao` 通道窗口内执行，登录态由通道窗口自己持有，不写 DPAPI Cookie。
    credentialKind: "webview",
    loginUrl: "https://mp.toutiao.com/profile_v4/graphic/publish",
    // 登录判据是「URL 落到 `/profile_v4` 编辑页」（见 `webview/toutiao.rs`），
    // 不依赖具体 Cookie 键；`loginWindow` 平台不会把该字段传给 `open_login_window`。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    loginWindow: true,
    available: true,
  },
  {
    id: "xiaohongshu",
    name: "小红书(图文)",
    credentialKind: "webview",
    loginUrl: "https://creator.xiaohongshu.com/",
    // 小红书没有独立登录页：未登录访问创作者中心会被重定向到 `/login`，
    // 扫码成功后跳回站内路由（`/new/home`、`/publish/publish`）。因此登录判据
    // 是「URL 离开 `/login` 且落在站内路由」（见 `webview/xiaohongshu.rs`），
    // 不依赖具体 Cookie 键；`loginWindow` 平台不会把该字段传给 `open_login_window`。
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    loginWindow: true,
    available: true,
  },
  {
    id: "douyin",
    name: "抖音",
    // 抖音创作者中心的草稿接口 `/web/api/media/aweme/draft` 不做签名校验，但
    // 依赖页面上下文（`creator.douyin.com` 自己的 Cookie 与站点特征）。Rust 侧
    // 直发会被风控拦成未登录，因此照头条的范式把 `fetch` 注入到长文编辑页里执行，
    // 登录态由 `channel-douyin` 通道窗口自己持有，不写 DPAPI Cookie。
    // 详见 `apps/desktop-app/src-tauri/src/platforms/webview/douyin.rs`。
    credentialKind: "webview",
    loginUrl: "https://creator.douyin.com/",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    loginWindow: true,
    available: true,
  },
];

export const PLATFORM_IDS: readonly string[] = PLATFORM_REGISTRY.map((p) => p.id);

export function findPlatform(id: string): PlatformDefinition | undefined {
  return PLATFORM_REGISTRY.find((p) => p.id === id);
}

export function platformName(id: string): string {
  return findPlatform(id)?.name ?? id;
}

export const ADAPTER_IMPLEMENTED_IDS: readonly string[] = [
  "cnblogs",
  "juejin",
  "csdn",
  "wechat",
  "zhihu",
  "jianshu",
  "segmentfault",
  "xiaohongshu",
  "51cto",
  "oschina",
  "tencentcloud",
  "aliyun",
  "baijiahao",
  "netease",
  "bilibili",
  "weibo",
  "toutiao",
  "douyin",
];

export function auditRegistry(): string[] {
  const issues: string[] = [];
  const ids = PLATFORM_REGISTRY.map((p) => p.id);
  const duplicates = ids.filter((id, index) => ids.indexOf(id) !== index);
  if (duplicates.length > 0) {
    issues.push(`平台 id 重复: ${[...new Set(duplicates)].join(", ")}`);
  }
  for (const id of ADAPTER_IMPLEMENTED_IDS) {
    const definition = findPlatform(id);
    if (!definition) {
      issues.push(`已实现的适配器 ${id} 未在注册表登记`);
    } else if (!definition.available) {
      issues.push(`已实现的适配器 ${id} 被标记为不可用`);
    }
  }
  for (const definition of PLATFORM_REGISTRY) {
    if (definition.available && !ADAPTER_IMPLEMENTED_IDS.includes(definition.id)) {
      issues.push(`平台 ${definition.id} 标记为可用但尚无适配器实现`);
    }
  }
  return issues;
}