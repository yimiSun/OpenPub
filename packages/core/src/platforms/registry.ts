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
    // 因此留空，改由登录窗口的 URL 跳转 / DOM 探针确认登录态。
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
    credentialKind: "cookie",
    loginUrl: "https://mp.toutiao.com/",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "direct",
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
    credentialKind: "webview",
    loginUrl: "https://creator.douyin.com/",
    requiredCookieKeys: [],
    region: "domestic",
    channelType: "webview",
    available: false,
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