export type CredentialKind = "metaweblog" | "cookie" | "webview";

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
  available: boolean;
}

export const PLATFORM_REGISTRY: readonly PlatformDefinition[] = [
  {
    id: "cnblogs",
    name: "博客园",
    credentialKind: "metaweblog",
    loginUrl: "https://i.cnblogs.com/settings",
    requiredCookieKeys: [],
    available: true,
  },
  {
    id: "juejin",
    name: "掘金",
    credentialKind: "cookie",
    loginUrl: "https://juejin.cn/",
    requiredCookieKeys: ["sessionid"],
    available: true,
  },
  {
    id: "zhihu",
    name: "知乎",
    credentialKind: "cookie",
    loginUrl: "https://www.zhihu.com/",
    requiredCookieKeys: [],
    available: false,
  },
  {
    id: "csdn",
    name: "CSDN",
    credentialKind: "cookie",
    loginUrl: "https://www.csdn.net/",
    requiredCookieKeys: ["UserToken"],
    available: true,
  },
  {
    id: "jianshu",
    name: "简书",
    credentialKind: "cookie",
    loginUrl: "https://www.jianshu.com/",
    requiredCookieKeys: [],
    available: false,
  },
  {
    id: "wechat",
    name: "微信公众号",
    credentialKind: "cookie",
    loginUrl: "https://mp.weixin.qq.com/",
    requiredCookieKeys: [],
    requiredConfigKeys: ["token"],
    available: true,
  },
  {
    id: "segmentfault",
    name: "思否",
    credentialKind: "cookie",
    loginUrl: "https://segmentfault.com/",
    requiredCookieKeys: [],
    available: false,
  },
  {
    id: "tencent",
    name: "腾讯云",
    credentialKind: "cookie",
    loginUrl: "https://cloud.tencent.com/",
    requiredCookieKeys: [],
    available: false,
  },
  {
    id: "aliyun",
    name: "阿里云",
    credentialKind: "cookie",
    loginUrl: "https://www.aliyun.com/",
    requiredCookieKeys: [],
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

export const ADAPTER_IMPLEMENTED_IDS: readonly string[] = ["cnblogs", "juejin", "csdn", "wechat"];

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