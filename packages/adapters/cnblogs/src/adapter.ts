import type { PlatformAdapter, PublishOptions, SyncResult } from "@openpub/core";

export interface CnblogsConfig {
  blogId: string;
  username: string;
  apiKey: string;
  metaWeblogUrl?: string;
}

const DEFAULT_META_WEBLOG_URL = "https://rpc.cnblogs.com/metaweblog/";

function buildXmlRpcCall(method: string, params: unknown[]): string {
  const paramXml = params
    .map((p) => {
      const value =
        typeof p === "string"
          ? `<value><string>${escapeXml(p)}</string></value>`
          : typeof p === "number"
            ? `<value><int>${p}</int></value>`
            : typeof p === "boolean"
              ? `<value><boolean>${p ? 1 : 0}</boolean></value>`
              : `<value><string>${escapeXml(String(p))}</string></value>`;
      return `<param>${value}</param>`;
    })
    .join("");

  return `<?xml version="1.0" encoding="UTF-8"?>
<methodCall>
  <methodName>${method}</methodName>
  <params>${paramXml}</params>
</methodCall>`;
}

function escapeXml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function parseXmlRpcResponse(xml: string): { fault?: string; value?: string } {
  const faultMatch = xml.match(/<fault>[\s\S]*?<string>([\s\S]*?)<\/string>/);
  if (faultMatch) {
    return { fault: faultMatch[1] };
  }

  const valueMatch = xml.match(/<value>[\s\S]*?<string>([\s\S]*?)<\/string>/);
  if (valueMatch) {
    return { value: valueMatch[1] };
  }

  const intMatch = xml.match(/<value>[\s\S]*?<int>(\d+)<\/int>/);
  if (intMatch) {
    return { value: intMatch[1] };
  }

  const booleanMatch = xml.match(/<value>[\s\S]*?<boolean>(\d)<\/boolean>/);
  if (booleanMatch) {
    return { value: booleanMatch[1] === "1" ? "true" : "false" };
  }

  return { value: xml };
}

async function callMetaWeblog(
  method: string,
  params: unknown[],
  config: CnblogsConfig,
): Promise<{ fault?: string; value?: string }> {
  const url = config.metaWeblogUrl ?? DEFAULT_META_WEBLOG_URL + config.blogId;
  const body = buildXmlRpcCall(method, params);

  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "text/xml",
    },
    body,
  });

  if (!response.ok) {
    throw new Error(`MetaWeblog HTTP ${response.status}: ${response.statusText}`);
  }

  const xml = await response.text();
  return parseXmlRpcResponse(xml);
}

export class CnblogsAdapter implements PlatformAdapter {
  readonly id = "cnblogs";
  readonly name = "博客园";
  readonly env: ("browser" | "api")[] = ["api"];

  private config: CnblogsConfig | null = null;

  configure(config: CnblogsConfig): void {
    this.config = config;
  }

  async checkAuth(): Promise<boolean> {
    if (!this.config) return false;
    try {
      const result = await callMetaWeblog(
        "blogger.getUsersBlogs",
        [this.config.apiKey, this.config.username, this.config.apiKey],
        this.config,
      );
      return !result.fault;
    } catch {
      return false;
    }
  }

  async preprocess?(options: PublishOptions): Promise<PublishOptions> {
    return options;
  }

  async uploadImages(images: string[], _context: unknown): Promise<string[]> {
    return images;
  }

  async sync(options: PublishOptions, _context: unknown): Promise<SyncResult> {
    if (!this.config) {
      return {
        success: false,
        platform: this.id,
        errorMsg: "未配置博客园 API 密钥，请先调用 configure()",
      };
    }

    try {
      const post = {
        title: options.title,
        description: options.markdown,
        categories: options.tags ?? [],
        mt_keywords: (options.tags ?? []).join(","),
      };

      const result = await callMetaWeblog(
        "metaWeblog.newPost",
        [this.config.blogId, this.config.username, this.config.apiKey, post, !options.isDraftOnly],
        this.config,
      );

      if (result.fault) {
        return {
          success: false,
          platform: this.id,
          errorMsg: `MetaWeblog 错误: ${result.fault}`,
        };
      }

      return {
        success: true,
        platform: this.id,
        draftUrl: `https://www.cnblogs.com/${this.config.blogId}/p/${result.value}.html`,
      };
    } catch (e) {
      return {
        success: false,
        platform: this.id,
        errorMsg: `发布失败: ${e instanceof Error ? e.message : String(e)}`,
      };
    }
  }
}

export async function testCnblogsPublish(
  config: CnblogsConfig,
  title: string,
  markdown: string,
): Promise<SyncResult> {
  const adapter = new CnblogsAdapter();
  adapter.configure(config);
  return adapter.sync({
    title,
    markdown,
    tags: ["OpenPub", "测试"],
    isDraftOnly: true,
  }, null);
}