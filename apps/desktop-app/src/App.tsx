import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Group, Panel, Separator } from "react-resizable-panels";
import "./App.css";

const PLATFORMS = [
  { id: "cnblogs", name: "博客园", configType: "metaweblog" as const },
  { id: "juejin", name: "掘金", configType: "cookie" as const },
  { id: "zhihu", name: "知乎", configType: "cookie" as const },
  { id: "csdn", name: "CSDN", configType: "cookie" as const },
  { id: "wechat", name: "微信公众号", configType: "webview" as const },
  { id: "segmentfault", name: "思否", configType: "cookie" as const },
  { id: "tencent", name: "腾讯云", configType: "cookie" as const },
  { id: "aliyun", name: "阿里云", configType: "cookie" as const },
];

const WEBVIEW_URLS: Record<string, string> = {
  wechat: "https://mp.weixin.qq.com/",
};

const INPUT_CLS =
  "w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2.5 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-500/30 transition-colors";

const TEXTAREA_CLS =
  "w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2.5 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-500/30 transition-colors font-mono resize-none";

interface CnblogsConfig {
  blogName: string;
  username: string;
  apiToken: string;
}

interface CookieConfig {
  cookie: string;
}

interface LogEntry {
  time: string;
  platform: string;
  message: string;
  success: boolean;
  editUrl?: string;
}

interface PublishResult {
  success: boolean;
  post_id: string;
  edit_url: string;
  message: string;
}

function App() {
  const [title, setTitle] = useState("OpenPub 测试文章");
  const [markdown, setMarkdown] = useState("# 你好 OpenPub\n\n这是你的第一篇文章。");
  const [selectedPlatforms, setSelectedPlatforms] = useState<Set<string>>(() => {
    try {
      const saved = localStorage.getItem("openpub_selected_platforms");
      return saved ? new Set(JSON.parse(saved) as string[]) : new Set<string>();
    } catch {
      return new Set<string>();
    }
  });
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [loginStatus, setLoginStatus] = useState<Record<string, string>>({});
  const [publishing, setPublishing] = useState(false);
  const [configPlatform, setConfigPlatform] = useState<string | null>(null);
  const [cnblogsConfig, setCnblogsConfig] = useState<CnblogsConfig>({ blogName: "", username: "", apiToken: "" });
  const [cookieConfigs, setCookieConfigs] = useState<Record<string, string>>({});
  const [configLoaded, setConfigLoaded] = useState(false);
  const logEndRef = useRef<HTMLDivElement>(null);
  const loginWindowsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    localStorage.setItem("openpub_selected_platforms", JSON.stringify([...selectedPlatforms]));
  }, [selectedPlatforms]);

  useEffect(() => {
    if (logEndRef.current) logEndRef.current.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  useEffect(() => {
    if (!configLoaded) {
      invoke<Record<string, string>>("load_platform_config", { platform: "cnblogs" })
        .then((c) => {
          if (c && c.blogName) {
            setCnblogsConfig({ blogName: c.blogName ?? "", username: c.username ?? "", apiToken: c.apiToken ?? "" });
            setLoginStatus((prev) => ({ ...prev, cnblogs: "已配置" }));
          }
        })
        .catch(() => {});

      const cookiePlatforms = PLATFORMS.filter((p) => p.configType === "cookie");
      for (const p of cookiePlatforms) {
        invoke<Record<string, string>>("load_platform_config", { platform: p.id })
          .then((c) => {
            if (c && c.cookie) {
              setCookieConfigs((prev) => ({ ...prev, [p.id]: c.cookie ?? "" }));
              setLoginStatus((prev) => ({ ...prev, [p.id]: "已配置" }));
            }
          })
          .catch(() => {});
      }

      setConfigLoaded(true);
    }
  }, [configLoaded]);

  const addLog = useCallback(
    (platform: string, message: string, success: boolean, editUrl?: string) => {
      const time = new Date().toLocaleTimeString("zh-CN", { hour12: false });
      setLogs((prev) => [...prev, { time, platform, message, success, editUrl }]);
    },
    [],
  );

  const togglePlatform = (id: string) => {
    setSelectedPlatforms((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleOpenWebview = async (e: React.MouseEvent, platformId: string, url: string, name: string) => {
    e.stopPropagation();
    const windowKey = `${platformId}-login`;
    if (loginWindowsRef.current.has(windowKey)) {
      addLog(name, "窗口已打开，请勿重复点击", true);
      return;
    }
    try {
      loginWindowsRef.current.add(windowKey);
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const webview = new WebviewWindow(windowKey, { url, title: `${name} - 登录`, width: 800, height: 700 });
      addLog(name, "登录窗口已打开", true);
      webview.once("tauri://destroyed", () => {
        loginWindowsRef.current.delete(windowKey);
        addLog(name, "登录窗口已关闭", true);
      });
    } catch (err) {
      loginWindowsRef.current.delete(windowKey);
      addLog(name, `打开窗口失败: ${err}`, false);
    }
  };

  const handleSaveCnblogsConfig = async () => {
    if (!cnblogsConfig.blogName.trim()) { addLog("博客园", "博客名不能为空", false); return; }
    if (!cnblogsConfig.username.trim()) { addLog("博客园", "用户名不能为空", false); return; }
    if (!cnblogsConfig.apiToken.trim()) { addLog("博客园", "访问令牌不能为空", false); return; }
    try {
      await invoke("save_platform_config", { platform: "cnblogs", config: cnblogsConfig });
      addLog("博客园", "配置已保存", true);
      setLoginStatus((prev) => ({ ...prev, cnblogs: "已配置" }));
      setConfigPlatform(null);
    } catch (e) { addLog("博客园", `保存失败: ${e}`, false); }
  };

  const handleSaveCookieConfig = async (platformId: string) => {
    const cookie = cookieConfigs[platformId] ?? "";
    if (!cookie.trim()) { addLog(PLATFORMS.find((p) => p.id === platformId)?.name ?? platformId, "Cookie 不能为空", false); return; }
    try {
      await invoke("save_platform_config", { platform: platformId, config: { cookie } });
      const name = PLATFORMS.find((p) => p.id === platformId)?.name ?? platformId;
      addLog(name, "配置已保存", true);
      setLoginStatus((prev) => ({ ...prev, [platformId]: "已配置" }));
      setConfigPlatform(null);
    } catch (e) { addLog(PLATFORMS.find((p) => p.id === platformId)?.name ?? platformId, `保存失败: ${e}`, false); }
  };

  const handleOpenUrl = async (e: React.MouseEvent, url: string, label: string) => {
    e.stopPropagation();
    try { await openUrl(url); } catch (err) { addLog(label, `打开浏览器失败: ${err}`, false); }
  };

  const handleOpenEditUrl = async (url: string) => {
    try { await openUrl(url); } catch (err) { addLog("系统", `打开链接失败: ${err}`, false); }
  };

  const handlePublishAll = async () => {
    if (selectedPlatforms.size === 0) { addLog("系统", "请先勾选至少一个平台", false); return; }
    if (!title.trim()) { addLog("系统", "文章标题不能为空", false); return; }
    setPublishing(true);
    for (const platformId of [...selectedPlatforms]) {
      const platform = PLATFORMS.find((p) => p.id === platformId);
      const platformName = platform?.name ?? platformId;
      try {
        const result = await invoke<PublishResult>("platform_publish", { platform: platformId, title, content: markdown });
        addLog(platformName, result.message, result.success, result.edit_url || undefined);
      } catch (e) { addLog(platformName, `发布失败: ${e}`, false); }
    }
    setPublishing(false);
  };

  const isConfigured = (id: string) => {
    if (id === "cnblogs") return !!(cnblogsConfig.blogName && cnblogsConfig.username && cnblogsConfig.apiToken);
    if (PLATFORMS.find((p) => p.id === id)?.configType === "cookie") return !!(cookieConfigs[id]);
    return false;
  };

  const LinkIcon = () => (
    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
    </svg>
  );

  const currentPlatform = PLATFORMS.find((p) => p.id === configPlatform);

  return (
    <div className="flex h-screen flex-col bg-zinc-950 text-zinc-100">
      {/* ── Header ── */}
      <header className="flex items-center justify-between border-b border-zinc-800/70 px-5 py-3">
        <h1 className="text-lg font-semibold tracking-tight text-emerald-400">OpenPub</h1>
        <div className="flex items-center gap-3 text-[11px] text-zinc-500">
          <span>已选 {selectedPlatforms.size} 个平台</span>
          {PLATFORMS.filter((p) => isConfigured(p.id)).map((p) => (
            <span key={p.id} className="text-emerald-500/70">· {p.name} ✓</span>
          ))}
        </div>
      </header>

      {/* ── Main ── */}
      <Group orientation="horizontal" className="flex flex-1 overflow-hidden">
        <Panel defaultSize={60} minSize={30}>
          <div className="flex h-full flex-col">
            <div className="border-b border-zinc-800/70 px-4 py-2 text-[11px] font-medium uppercase tracking-wider text-zinc-500">
              Markdown 编辑器
            </div>
            <div className="border-b border-zinc-800/40 px-4 py-2">
              <input
                type="text" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="文章标题"
                className="w-full bg-transparent text-sm font-medium text-zinc-200 placeholder-zinc-600 focus:outline-none"
              />
            </div>
            <textarea
              className="flex-1 resize-none bg-zinc-950 p-4 font-mono text-sm leading-relaxed text-zinc-200 placeholder-zinc-600 focus:outline-none"
              placeholder="在此输入文章内容..." value={markdown} onChange={(e) => setMarkdown(e.target.value)}
            />
          </div>
        </Panel>

        <Separator className="group relative flex w-1.5 items-center justify-center bg-zinc-900 transition-colors hover:bg-emerald-500/20 data-[resize-handle-active]:bg-emerald-500/40">
          <div className="h-8 w-0.5 rounded-full bg-zinc-700 transition-colors group-hover:bg-emerald-500/60 group-data-[resize-handle-active]:bg-emerald-500" />
        </Separator>

        <Panel defaultSize={40} minSize={20}>
          <div className="flex h-full flex-col">
            <div className="border-b border-zinc-800/70 px-4 py-2 text-[11px] font-medium uppercase tracking-wider text-zinc-500">
              平台选择
            </div>
            <div className="flex-1 overflow-y-auto overflow-x-auto min-w-0 px-3 py-2">
              {PLATFORMS.map((p) => (
                <div key={p.id} className="group flex items-center justify-between rounded-lg px-3 py-2.5 transition-colors hover:bg-zinc-800/50 min-w-0">
                  <label className="flex cursor-pointer items-center gap-2.5">
                    <input
                      type="checkbox" checked={selectedPlatforms.has(p.id)} onChange={() => togglePlatform(p.id)}
                      className="h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-800 text-emerald-500 focus:ring-emerald-500/30"
                    />
                    <span className="truncate text-sm text-zinc-200">{p.name}</span>
                  </label>
                  <div className="flex shrink-0 items-center gap-2">
                    <span className={`text-[11px] font-medium ${loginStatus[p.id] ? "text-emerald-400" : "text-zinc-500"}`}>
                      {loginStatus[p.id] ?? "未配置"}
                    </span>
                    <button
                      onClick={(e) => { e.stopPropagation(); setConfigPlatform(p.id); }}
                      className="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-400 transition-colors hover:border-amber-500/50 hover:bg-amber-500/20"
                    >
                      {p.configType === "webview" ? "登录" : "配置"}
                    </button>
                  </div>
                </div>
              ))}
            </div>
            <div className="border-t border-zinc-800/70 p-3">
              <button
                onClick={handlePublishAll}
                disabled={publishing || selectedPlatforms.size === 0}
                className={`w-full rounded-lg bg-emerald-600 py-2.5 text-sm font-semibold text-white transition-all hover:bg-emerald-500 disabled:cursor-not-allowed ${publishing || selectedPlatforms.size === 0 ? "opacity-30" : ""}`}
              >
                {publishing ? "同步中..." : selectedPlatforms.size > 0 ? `一键同步草稿 (${selectedPlatforms.size})` : "请先勾选平台"}
              </button>
            </div>
          </div>
        </Panel>
      </Group>

      {/* ── Cnblogs Config Modal ── */}
      {configPlatform === "cnblogs" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setConfigPlatform(null)}>
          <div className="w-[440px] rounded-xl border border-zinc-800 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-zinc-100">博客园 MetaWeblog 配置</h2>
            <p className="mt-1 text-xs text-zinc-500">配置后可通过 MetaWeblog API 将文章发布到博客园（草稿模式）</p>
            <div className="mt-4 flex items-center gap-2 rounded-lg bg-zinc-900 px-3 py-2">
              <span className="text-[11px] font-medium text-zinc-500">端点</span>
              <span className="truncate text-[11px] font-mono text-emerald-400/80">
                {cnblogsConfig.blogName ? `https://rpc.cnblogs.com/metaweblog/${cnblogsConfig.blogName}` : "填写博客名后自动生成"}
              </span>
            </div>
            <div className="mt-5 space-y-4">
              <div>
                <label className="mb-1.5 block text-xs font-medium text-zinc-300">博客名</label>
                <input type="text" className={INPUT_CLS} placeholder="你的博客名（如 my-blog）" value={cnblogsConfig.blogName} onChange={(e) => setCnblogsConfig((p) => ({ ...p, blogName: e.target.value }))} />
                <p className="mt-1.5 text-[11px] text-zinc-500">即 https://www.cnblogs.com/<span className="text-emerald-400/70">你的博客名</span>/ 中的部分</p>
              </div>
              <div>
                <label className="mb-1.5 block text-xs font-medium text-zinc-300">用户名</label>
                <input type="text" className={INPUT_CLS} placeholder="博客园登录用户名" value={cnblogsConfig.username} onChange={(e) => setCnblogsConfig((p) => ({ ...p, username: e.target.value }))} />
              </div>
              <div>
                <label className="mb-1.5 block text-xs font-medium text-zinc-300">访问令牌</label>
                <input type="password" className={INPUT_CLS} placeholder="MetaWeblog 访问令牌" value={cnblogsConfig.apiToken} onChange={(e) => setCnblogsConfig((p) => ({ ...p, apiToken: e.target.value }))} />
                <div className="mt-2 rounded-lg bg-emerald-500/[0.07] px-3 py-2.5">
                  <p className="text-[11px] leading-relaxed text-zinc-400">
                    <span className="font-medium text-emerald-400/80">如何获取？</span> 前往博客园后台 → 设置 → 其他设置，勾选「允许通过 MetaWeblog API 发表随笔」，并在下方获取访问令牌。
                  </p>
                  <button onClick={(e) => handleOpenUrl(e, "https://i.cnblogs.com/settings", "博客园")} className="mt-2 inline-flex items-center gap-1.5 rounded-md text-[11px] font-medium text-emerald-400 transition-colors hover:text-emerald-300">
                    <LinkIcon /> 前往博客园获取
                  </button>
                </div>
              </div>
            </div>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button onClick={() => setConfigPlatform(null)} className="rounded-md px-4 py-2 text-sm font-medium text-zinc-400 transition-colors hover:text-zinc-100">取消</button>
              <button onClick={handleSaveCnblogsConfig} className="rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500">保存配置</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Cookie Config Modal (掘金/知乎/CSDN/思否/腾讯云/阿里云) ── */}
      {configPlatform && currentPlatform?.configType === "cookie" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setConfigPlatform(null)}>
          <div className="w-[440px] rounded-xl border border-zinc-800 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-zinc-100">{currentPlatform.name} Cookie 配置</h2>
            <p className="mt-1 text-xs text-zinc-500">通过 Cookie 鉴权调用 {currentPlatform.name} API 创建草稿</p>
            <div className="mt-5 space-y-4">
              <div>
                <label className="mb-1.5 block text-xs font-medium text-zinc-300">Cookie 字符串</label>
                <textarea
                  className={TEXTAREA_CLS + " h-28"}
                  placeholder={`从浏览器开发者工具中复制 ${currentPlatform.name} 的完整 Cookie 字符串`}
                  value={cookieConfigs[configPlatform] ?? ""}
                  onChange={(e) => setCookieConfigs((prev) => ({ ...prev, [configPlatform!]: e.target.value }))}
                />
                <div className="mt-2 rounded-lg bg-emerald-500/[0.07] px-3 py-2.5">
                  <p className="text-[11px] leading-relaxed text-zinc-400">
                    <span className="font-medium text-emerald-400/80">如何获取？</span>{" "}
                    在浏览器中登录{currentPlatform.name}，按 F12 打开开发者工具 → Network → 任意请求 → Headers → Cookie，复制完整值粘贴到上方。
                  </p>
                  <button
                    onClick={(e) => handleOpenUrl(e, `https://${currentPlatform.id === "csdn" ? "csdn.net" : currentPlatform.id === "zhihu" ? "zhihu.com" : currentPlatform.id === "juejin" ? "juejin.cn" : currentPlatform.id === "segmentfault" ? "segmentfault.com" : currentPlatform.id === "tencent" ? "cloud.tencent.com" : "aliyun.com"}`, currentPlatform.name)}
                    className="mt-2 inline-flex items-center gap-1.5 rounded-md text-[11px] font-medium text-emerald-400 transition-colors hover:text-emerald-300"
                  >
                    <LinkIcon /> 前往{currentPlatform.name}登录
                  </button>
                </div>
              </div>
            </div>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button onClick={() => setConfigPlatform(null)} className="rounded-md px-4 py-2 text-sm font-medium text-zinc-400 transition-colors hover:text-zinc-100">取消</button>
              <button onClick={() => handleSaveCookieConfig(configPlatform!)} className="rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500">保存配置</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Webview Login Modal (微信公众号) ── */}
      {configPlatform && currentPlatform?.configType === "webview" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setConfigPlatform(null)}>
          <div className="w-[380px] rounded-xl border border-zinc-800 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-zinc-100">{currentPlatform.name} 登录</h2>
            <p className="mt-1 text-xs text-zinc-500">在新窗口中打开{currentPlatform.name}官方登录页进行扫码登录</p>
            <div className="mt-5">
              <button
                onClick={(e) => handleOpenWebview(e, configPlatform!, WEBVIEW_URLS[configPlatform!] ?? "https://example.com", currentPlatform.name)}
                className="w-full rounded-lg border border-emerald-500/30 bg-emerald-500/10 py-3 text-sm font-medium text-emerald-400 transition-colors hover:border-emerald-500/50 hover:bg-emerald-500/20"
              >
                打开{currentPlatform.name}扫码登录
              </button>
              <p className="mt-3 text-center text-[11px] text-zinc-500">登录完成后关闭窗口即可</p>
            </div>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button onClick={() => setConfigPlatform(null)} className="rounded-md px-4 py-2 text-sm font-medium text-zinc-400 transition-colors hover:text-zinc-100">关闭</button>
            </div>
          </div>
        </div>
      )}

      {/* ── Log Panel ── */}
      <div className="flex h-40 flex-col border-t border-zinc-800/70">
        <div className="border-b border-zinc-800/70 px-4 py-1.5 text-[11px] font-medium uppercase tracking-wider text-zinc-500">同步日志</div>
        <div className="flex-1 overflow-y-auto bg-zinc-950 px-4 py-1.5 font-mono text-xs">
          {logs.length === 0 ? (
            <span className="text-zinc-600">暂无日志。勾选平台后点击「一键同步草稿」开始。</span>
          ) : (
            <>
              {logs.map((log, i) => (
                <div key={i} className="py-0.5">
                  <span className="text-zinc-600">[{log.time}]</span>{" "}
                  <span className="text-emerald-400/80">{log.platform}</span>{" "}
                  <span className={log.success ? "text-emerald-400" : "text-red-400"}>{log.message}</span>
                  {log.editUrl && (
                    <>
                      {" "}
                      <button
                        onClick={() => handleOpenEditUrl(log.editUrl!)}
                        className="inline-flex items-center gap-1 rounded text-[11px] text-emerald-400/70 underline decoration-emerald-400/30 underline-offset-2 transition-colors hover:text-emerald-300 hover:decoration-emerald-300"
                      >
                        <svg className="h-2.5 w-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                          <path strokeLinecap="round" strokeLinejoin="round" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                        </svg>
                        查看草稿
                      </button>
                    </>
                  )}
                </div>
              ))}
              <div ref={logEndRef} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

export default App;