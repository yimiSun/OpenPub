import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { Group, Panel, Separator } from "react-resizable-panels";
import {
  PLATFORM_REGISTRY,
  articleLabel,
  findUsableSyncEntry,
  auditRegistry,
  type Article,
  type PublishResult,
  type SyncLogEntry,
  type SyncMapRecord,
} from "@openpub/core";
import { AboutDialog } from "./components/AboutDialog";
import { SettingsPanel } from "./components/SettingsPanel";
import { Toast, useToast } from "./components/Toast";
import "./App.css";

const PLATFORMS = PLATFORM_REGISTRY.map((p) => ({
  id: p.id,
  name: p.name,
  configType: p.credentialKind,
  loginUrl: p.loginUrl,
  requiredCookieKeys: p.requiredCookieKeys,
  requiredConfigKeys: p.requiredConfigKeys ?? [],
  available: p.available,
}));

const LOGIN_SUCCESS_EVENT = "login-success";
const LOGIN_FAILED_EVENT = "login-failed";

const INPUT_CLS =
  "w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2.5 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-500/30 transition-colors";

const TEXTAREA_CLS =
  "w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2.5 text-sm text-zinc-100 placeholder-zinc-500 focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-500/30 transition-colors font-mono resize-none";

/**
 * 勾选「覆盖已发布」时的提示文案，按平台各自的实际行为写，不能一句话套用。
 * 掘金是「存草稿、需手动发布」；CSDN 已支持提交发布（进审核队列），因此不再需要提示。
 */
const OVERWRITE_HINTS: Record<string, string> = {
  juejin: "掘金目前仅支持保存草稿，请到掘金后台手动发布。",
};

interface CnblogsConfig {
  blogName: string;
  username: string;
  apiToken: string;
}

function App({ onReady }: { onReady?: () => void }) {
  const [articles, setArticles] = useState<Article[]>([]);
  const [currentArticleId, setCurrentArticleId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [markdown, setMarkdown] = useState("");
  const [articlesLoaded, setArticlesLoaded] = useState(false);
  const [syncMap, setSyncMap] = useState<SyncMapRecord>({});
  const [publishOverwrite, setPublishOverwrite] = useState(false);
  const [previewMode, setPreviewMode] = useState(false);
  const [pendingConfirm, setPendingConfirm] = useState<{ platforms: string[] } | null>(null);
  const [pendingConfigWarn, setPendingConfigWarn] = useState<{ unconfigured: string[]; names: string[] } | null>(null);
  const [selectedPlatforms, setSelectedPlatforms] = useState<Set<string>>(() => {
    try {
      const saved = localStorage.getItem("openpub_selected_platforms");
      if (!saved) return new Set<string>();
      const available = new Set(PLATFORMS.filter((p) => p.available).map((p) => p.id));
      return new Set((JSON.parse(saved) as string[]).filter((id) => available.has(id)));
    } catch {
      return new Set<string>();
    }
  });
  const overwriteHints = [...selectedPlatforms]
    .map((id) => OVERWRITE_HINTS[id])
    .filter((text) => text !== undefined);

  const [logs, setLogs] = useState<SyncLogEntry[]>([]);
  const [loginStatus, setLoginStatus] = useState<Record<string, string>>({});
  const [publishing, setPublishing] = useState(false);
  const [exportingLogs, setExportingLogs] = useState(false);
  const [runningPlatforms, setRunningPlatforms] = useState<string[]>([]);
  const [configPlatform, setConfigPlatform] = useState<string | null>(null);
  const [manualCookieOpen, setManualCookieOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { toast, showToast, dismissToast } = useToast();
  const [cnblogsConfig, setCnblogsConfig] = useState<CnblogsConfig>({ blogName: "", username: "", apiToken: "" });
  const [cookieConfigs, setCookieConfigs] = useState<Record<string, string>>({});
  const [configTokens, setConfigTokens] = useState<Record<string, string>>({});
  const [configLoaded, setConfigLoaded] = useState(false);
  const logEndRef = useRef<HTMLDivElement>(null);
  const titleInputRef = useRef<HTMLInputElement>(null);
  const loginWindowsRef = useRef<Set<string>>(new Set());
  const readyNotifiedRef = useRef(false);

  useEffect(() => {
    if (readyNotifiedRef.current || !articlesLoaded) return;
    readyNotifiedRef.current = true;
    onReady?.();
  }, [articlesLoaded, onReady]);

  useEffect(() => {
    localStorage.setItem("openpub_selected_platforms", JSON.stringify([...selectedPlatforms]));
  }, [selectedPlatforms]);

  useEffect(() => {
    const issues = auditRegistry();
    issues.forEach((message) =>
      setLogs((prev) => [
        ...prev,
        {
          timestamp: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
          articleId: "",
          platformId: "system",
          platformName: "系统",
          stage: "audit",
          outcome: "failure",
          message,
        },
      ]),
    );
  }, []);

  useEffect(() => {
    if (logEndRef.current) logEndRef.current.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  useEffect(() => {
    if (articlesLoaded) return;
    invoke<Article[]>("list_articles")
      .then((list) => {
        setArticles(list);
        const savedId = localStorage.getItem("openpub_current_article");
        const target = list.find((a) => a.id === savedId) ?? list[0];
        if (target) {
          setCurrentArticleId(target.id);
          setTitle(target.title);
          setMarkdown(target.content);
        }
      })
      .catch(() => {})
      .finally(() => setArticlesLoaded(true));
  }, [articlesLoaded]);

  useEffect(() => {
    if (!currentArticleId) {
      setSyncMap({});
      return;
    }
    localStorage.setItem("openpub_current_article", currentArticleId);
    invoke<SyncMapRecord>("load_sync_map", { articleId: currentArticleId })
      .then((map) => setSyncMap(map ?? {}))
      .catch(() => setSyncMap({}));
  }, [currentArticleId]);

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

      const cookiePlatforms = PLATFORMS.filter((p) => p.configType === "cookie" && p.available);
      for (const p of cookiePlatforms) {
        invoke<string | null>("load_cookie", { platform: p.id })
          .then((stored) => {
            if (stored && stored.trim()) {
              setLoginStatus((prev) => ({ ...prev, [p.id]: "已登录" }));
            }
          })
          .catch(() => {});

        invoke<Record<string, string>>("load_platform_config", { platform: p.id })
          .then((c) => {
            if (c && c.cookie) {
              setCookieConfigs((prev) => ({ ...prev, [p.id]: c.cookie ?? "" }));
              setLoginStatus((prev) => ({ ...prev, [p.id]: "已配置" }));
            }
            if (c && c.token) {
              setConfigTokens((prev) => ({ ...prev, [p.id]: c.token ?? "" }));
            }
          })
          .catch(() => {});
      }

      setConfigLoaded(true);
    }
  }, [configLoaded]);

  const addLog = useCallback((entry: SyncLogEntry) => {
    setLogs((prev) => [...prev, entry]);
  }, []);

  const pushLog = useCallback(
    (
      platformId: string,
      message: string,
      outcome: SyncLogEntry["outcome"],
      extra?: Partial<SyncLogEntry>,
    ) => {
      addLog({
        timestamp: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
        articleId: currentArticleId ?? "",
        platformId,
        platformName: PLATFORMS.find((p) => p.id === platformId)?.name ?? platformId,
        stage: extra?.stage ?? "sync",
        outcome,
        message,
        ...extra,
      });
    },
    [addLog, currentArticleId],
  );

  useEffect(() => {
    setManualCookieOpen(false);
  }, [configPlatform]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const register = (event: string, handler: (payload: unknown) => void) => {
      listen(event, (e) => handler(e.payload))
        .then((unlisten) => {
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        })
        .catch(() => {});
    };

    register(LOGIN_SUCCESS_EVENT, (payload) => {
      const platformId = (payload as { platform?: string })?.platform ?? "";
      if (!platformId) return;
      const name = PLATFORMS.find((p) => p.id === platformId)?.name ?? platformId;
      loginWindowsRef.current.delete(platformId);
      setLoginStatus((prev) => ({ ...prev, [platformId]: "已登录" }));
      setConfigPlatform((current) => (current === platformId ? null : current));
      pushLog(platformId, `${name} 登录成功，凭据已加密保存`, "success", { stage: "login" });
      showToast(`${name} 登录成功`);
    });

    register(LOGIN_FAILED_EVENT, (payload) => {
      const data = payload as { platform?: string; message?: string };
      const platformId = data?.platform ?? "";
      if (!platformId) return;
      const message = data?.message ?? "登录失败";
      loginWindowsRef.current.delete(platformId);
      pushLog(platformId, message, "failure", { stage: "login" });
      showToast(message, "error");
    });

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [pushLog, showToast]);

  const handleExportLogs = useCallback(async () => {
    if (logs.length === 0) {
      pushLog("system", "当前没有可导出的日志", "failure", { stage: "export" });
      return;
    }
    setExportingLogs(true);
    try {
      const jsonl = logs.map((entry) => JSON.stringify(entry)).join("\n");
      const path = await invoke<string>("export_sync_log", { content: jsonl });
      pushLog("system", `日志已导出：${path}`, "success", { stage: "export" });
    } catch (err) {
      pushLog("system", `导出日志失败: ${err}`, "failure", { stage: "export" });
    } finally {
      setExportingLogs(false);
    }
  }, [logs, pushLog]);

  const togglePlatform = (id: string) => {
    const platform = PLATFORMS.find((p) => p.id === id);
    if (platform && !platform.available) {
      showToast(`${platform.name} 的适配器开发中，暂不支持同步`, "error");
      return;
    }
    setSelectedPlatforms((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handlePlatformConfigClick = (e: React.MouseEvent, platformId: string) => {
    e.stopPropagation();
    const platform = PLATFORMS.find((p) => p.id === platformId);
    if (!platform) return;
    if (!platform.available) {
      showToast(`${platform.name} 的适配器开发中，暂不支持配置`, "error");
      return;
    }
    setConfigPlatform(platformId);
  };

  const persistCurrent = useCallback(
    async (nextTitle: string, nextContent: string) => {
      if (!currentArticleId) return;
      try {
        await invoke<Article>("update_article", {
          id: currentArticleId,
          title: nextTitle,
          content: nextContent,
        });
        setArticles((prev) =>
          prev.map((a) => (a.id === currentArticleId ? { ...a, title: nextTitle, content: nextContent } : a)),
        );
      } catch (err) {
        pushLog("system", `保存文章失败: ${err}`, "failure", { stage: "article", platformName: "系统" });
      }
    },
    [currentArticleId, pushLog],
  );

  const cleanupEmptyArticle = useCallback(async () => {
    if (!currentArticleId) return;
    const trimmedTitle = title.trim();
    const trimmedContent = markdown.trim();
    if (trimmedTitle !== "" || trimmedContent !== "") return;
    if (articles.length <= 1) return;
    try {
      await invoke("delete_article", { id: currentArticleId });
      setArticles((prev) => prev.filter((a) => a.id !== currentArticleId));
    } catch {
      /* 静默清理失败不影响切换 */
    }
  }, [currentArticleId, title, markdown, articles.length]);

  const handleCreateArticle = async () => {
    if (currentArticleId) {
      await persistCurrent(title, markdown);
      await cleanupEmptyArticle();
    }
    try {
      const created = await invoke<Article>("create_article", { title: "", content: "" });
      setArticles((prev) => [...prev, created]);
      setCurrentArticleId(created.id);
      setTitle(created.title);
      setMarkdown(created.content);
      setSyncMap({});
      setPreviewMode(false);
      queueMicrotask(() => titleInputRef.current?.focus());
    } catch (err) {
      pushLog("system", `新建文章失败: ${err}`, "failure", { stage: "article", platformName: "系统" });
    }
  };

  const handleSelectArticle = async (article: Article) => {
    if (article.id === currentArticleId) return;
    if (currentArticleId) {
      await persistCurrent(title, markdown);
      await cleanupEmptyArticle();
    }
    setCurrentArticleId(article.id);
    setTitle(article.title);
    setMarkdown(article.content);
    setPreviewMode(false);
  };

  const handleDeleteArticle = async (article: Article) => {
    const confirmed = window.confirm(
      `确认删除文章「${articleLabel(article)}」？该文章在各平台的同步映射也会一并删除。`,
    );
    if (!confirmed) return;
    try {
      await invoke("delete_article", { id: article.id });
      const remaining = articles.filter((a) => a.id !== article.id);
      setArticles(remaining);
      if (currentArticleId === article.id) {
        const next = remaining[0];
        setCurrentArticleId(next?.id ?? null);
        setTitle(next?.title ?? "");
        setMarkdown(next?.content ?? "");
        setSyncMap({});
      }
    } catch (err) {
      pushLog("system", `删除文章失败: ${err}`, "failure", { stage: "article", platformName: "系统" });
    }
  };

  const handleOpenLogin = async (e: React.MouseEvent, platformId: string) => {
    e.stopPropagation();
    const platform = PLATFORMS.find((p) => p.id === platformId);
    if (!platform) return;
    if (!platform.available) {
      showToast(`${platform.name} 的适配器开发中，暂不支持一键登录`, "error");
      return;
    }
    if (loginWindowsRef.current.has(platformId)) {
      pushLog(platformId, "登录窗口已打开，请勿重复点击", "success", { stage: "login" });
      return;
    }
    try {
      loginWindowsRef.current.add(platformId);
      await invoke<string>("open_login_window", {
        platform: platformId,
        loginUrl: platform.loginUrl,
        requiredCookieKeys: platform.requiredCookieKeys,
      });
      pushLog(platformId, "登录窗口已打开，请完成扫码", "success", { stage: "login" });
    } catch (err) {
      loginWindowsRef.current.delete(platformId);
      pushLog(platformId, `打开登录窗口失败: ${err}`, "failure", { stage: "login" });
      showToast(`打开登录窗口失败: ${err}`, "error");
    }
  };

  const handleSaveCnblogsConfig = async () => {
    if (!cnblogsConfig.blogName.trim()) { pushLog("cnblogs", "博客名不能为空", "failure", { stage: "config" }); return; }
    if (!cnblogsConfig.username.trim()) { pushLog("cnblogs", "用户名不能为空", "failure", { stage: "config" }); return; }
    if (!cnblogsConfig.apiToken.trim()) { pushLog("cnblogs", "访问令牌不能为空", "failure", { stage: "config" }); return; }
    try {
      await invoke("save_platform_config", { platform: "cnblogs", config: cnblogsConfig });
      pushLog("cnblogs", "配置已保存", "success", { stage: "config" });
      setLoginStatus((prev) => ({ ...prev, cnblogs: "已配置" }));
      setConfigPlatform(null);
    } catch (e) { pushLog("cnblogs", `保存失败: ${e}`, "failure", { stage: "config" }); }
  };

  const handleSaveCookieConfig = async (platformId: string) => {
    const platform = PLATFORMS.find((p) => p.id === platformId);
    const cookie = cookieConfigs[platformId] ?? "";
    if (!cookie.trim()) { pushLog(platformId, "Cookie 不能为空", "failure", { stage: "config" }); return; }
    const extraKeys = platform?.requiredConfigKeys ?? [];
    const config: Record<string, string> = { cookie };
    if (extraKeys.includes("token")) {
      const token = (configTokens[platformId] ?? "").trim();
      if (!token) { pushLog(platformId, "token 不能为空（在公众号后台地址栏的 token 参数里获取）", "failure", { stage: "config" }); return; }
      config.token = token;
    }
    try {
      await invoke("save_platform_config", { platform: platformId, config });
      pushLog(platformId, "配置已保存", "success", { stage: "config" });
      setLoginStatus((prev) => ({ ...prev, [platformId]: "已配置" }));
      setConfigPlatform(null);
    } catch (e) { pushLog(platformId, `保存失败: ${e}`, "failure", { stage: "config" }); }
  };

  const handleOpenUrl = async (e: React.MouseEvent, url: string, label: string) => {
    e.stopPropagation();
    try { await openUrl(url); } catch (err) { pushLog("system", `打开浏览器失败: ${err}`, "failure", { stage: "system", platformName: label }); }
  };

  const handleOpenEditUrl = async (url: string) => {
    try { await openUrl(url); } catch (err) { pushLog("system", `打开链接失败: ${err}`, "failure", { stage: "system" }); }
  };

  const runPublish = async (platformIds: string[], overwrite: boolean) => {
    if (!currentArticleId) return;
    setPublishing(true);
    setRunningPlatforms(platformIds);
    for (const platformId of platformIds) {
      const entry = findUsableSyncEntry(syncMap, platformId);
      const isUpdate = entry !== undefined;
      setRunningPlatforms((prev) => prev.filter((id) => id !== platformId));
      pushLog(platformId, isUpdate ? "开始更新已同步内容..." : "开始创建草稿...", "running", {
        stage: "sync",
        operation: isUpdate ? "update" : "create",
        isPublished: overwrite,
      });
      try {
        const result = await invoke<PublishResult>("platform_publish", {
          platform: platformId,
          title,
          content: markdown,
          articleId: currentArticleId,
          existingId: isUpdate ? entry!.id : null,
          isUpdate,
          publishStatus: overwrite,
          existingPublished: isUpdate ? entry!.published : false,
        });
        pushLog(platformId, result.message, result.success ? "success" : "failure", {
          stage: "sync",
          failureClass: result.failure_class,
          draftUrl: result.edit_url || undefined,
          operation: result.operation,
          isPublished: result.isPublished,
        });
        if (result.success) {
          const refreshed = await invoke<SyncMapRecord>("load_sync_map", { articleId: currentArticleId });
          setSyncMap(refreshed ?? {});
        }
      } catch (e) {
        pushLog(platformId, `发布失败: ${e}`, "failure", { stage: "sync", operation: isUpdate ? "update" : "create" });
      }
    }
    setRunningPlatforms([]);
    setPublishing(false);
  };

  const handlePublishAll = async () => {
    if (!currentArticleId) { pushLog("system", "请先新建或选择一篇文章", "failure", { stage: "system" }); return; }
    if (selectedPlatforms.size === 0) { pushLog("system", "请先勾选至少一个平台", "failure", { stage: "system" }); return; }

    const unconfigured: string[] = [];
    const unconfigNames: string[] = [];
    for (const id of selectedPlatforms) {
      if (!isConfigured(id)) {
        unconfigured.push(id);
        unconfigNames.push(PLATFORMS.find((p) => p.id === id)?.name ?? id);
      }
    }
    if (unconfigured.length > 0) {
      setPendingConfigWarn({ unconfigured, names: unconfigNames });
      return;
    }

    const targets = [...selectedPlatforms];
    await persistCurrent(title, markdown);

    if (publishOverwrite) {
      setPendingConfirm({ platforms: targets });
      return;
    }

    await runPublish(targets, false);
  };

  const handleConfirmOverwrite = async () => {
    if (!pendingConfirm) return;
    const targets = pendingConfirm.platforms;
    setPendingConfirm(null);
    await runPublish(targets, true);
  };

  const isConfigured = (id: string) => {
    if (id === "cnblogs") return !!(cnblogsConfig.blogName && cnblogsConfig.username && cnblogsConfig.apiToken);
    const platform = PLATFORMS.find((p) => p.id === id);
    if (platform?.configType === "cookie") {
      if (!cookieConfigs[id]) return false;
      for (const key of platform.requiredConfigKeys) {
        if (key === "token" && !(configTokens[id] ?? "").trim()) return false;
      }
      return true;
    }
    return false;
  };

  const LinkIcon = () => (
    <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
      <path strokeLinecap="round" strokeLinejoin="round" d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
    </svg>
  );

  const handleDataChanged = useCallback(() => {
    localStorage.removeItem("openpub_current_article");
    setArticles([]);
    setCurrentArticleId(null);
    setTitle("");
    setMarkdown("");
    setSyncMap({});
    setLoginStatus({});
    setCookieConfigs({});
    setCnblogsConfig({ blogName: "", username: "", apiToken: "" });
    setLogs([]);
    setArticlesLoaded(false);
    setConfigLoaded(false);
  }, []);

  const currentPlatform = PLATFORMS.find((p) => p.id === configPlatform);
  const manualOnlyPlatform = (currentPlatform?.requiredConfigKeys ?? []).length > 0;
  const hasContent = !!(currentArticleId && markdown.trim());

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
          <button
            onClick={() => setSettingsOpen(true)}
            className="ml-1 inline-flex items-center gap-1 rounded-md border border-zinc-700 px-2.5 py-1 text-[11px] font-medium text-zinc-400 transition-colors hover:border-emerald-500/50 hover:text-emerald-400"
          >
            <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37a1.724 1.724 0 002.572-1.065z" />
              <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
            设置
          </button>
          <button
            onClick={() => setAboutOpen(true)}
            className="ml-1 rounded-md border border-zinc-700 px-2.5 py-1 text-[11px] font-medium text-zinc-400 transition-colors hover:border-emerald-500/50 hover:text-emerald-400"
          >
            关于 / 支持
          </button>
        </div>
      </header>

      {/* ── Main ── */}
      <Group orientation="horizontal" className="flex min-h-0 flex-1 overflow-hidden">
        <Panel defaultSize="65%" minSize="30%">
          <div className="flex h-full min-h-0 min-w-0 flex-col">
            <div className="flex items-center justify-between border-b border-zinc-800/70 px-4 py-2">
              <span className="text-[11px] font-medium uppercase tracking-wider text-zinc-500">
                文章列表 ({articles.length})
              </span>
              <button
                onClick={handleCreateArticle}
                className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-400 transition-colors hover:border-emerald-500/50 hover:bg-emerald-500/20"
              >
                + 新建文章
              </button>
            </div>
            <div className="max-h-40 shrink-0 overflow-y-auto border-b border-zinc-800/40 px-2 py-1.5">
              {articles.length === 0 ? (
                <div className="px-2 py-3 text-center text-[11px] leading-relaxed text-zinc-500">
                  还没有任何文章。点击右上角「+ 新建文章」开始写作，之后每篇文章都会独立记录各平台的同步状态。
                </div>
              ) : (
                articles.map((a) => {
                  const active = a.id === currentArticleId;
                  return (
                    <div
                      key={a.id}
                      onClick={() => handleSelectArticle(a)}
                      className={`group flex cursor-pointer items-center justify-between rounded-md px-2.5 py-1.5 transition-colors ${
                        active ? "bg-emerald-500/10" : "hover:bg-zinc-800/50"
                      }`}
                    >
                      <span
                        className={`truncate text-xs ${active ? "font-medium text-emerald-400" : "text-zinc-300"}`}
                        title={articleLabel(a)}
                      >
                        {articleLabel(a)}
                      </span>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteArticle(a);
                        }}
                        className="ml-2 shrink-0 rounded px-1 text-[10px] text-zinc-600 opacity-0 transition-colors hover:text-red-400 group-hover:opacity-100"
                      >
                        删除
                      </button>
                    </div>
                  );
                })
              )}
            </div>
            <div className="flex items-center justify-between border-b border-zinc-800/70 px-4 py-2">
              <span className="text-[11px] font-medium uppercase tracking-wider text-zinc-500">
                当前文章
              </span>
              <div className="flex items-center rounded-md border border-zinc-700 bg-zinc-900 p-0.5">
                <button
                  onClick={() => setPreviewMode(false)}
                  className={`rounded px-2.5 py-0.5 text-[11px] font-medium transition-colors ${
                    !previewMode ? "bg-emerald-600 text-white" : "text-zinc-500 hover:text-zinc-300"
                  }`}
                >
                  编辑
                </button>
                <button
                  onClick={() => setPreviewMode(true)}
                  className={`rounded px-2.5 py-0.5 text-[11px] font-medium transition-colors ${
                    previewMode ? "bg-emerald-600 text-white" : "text-zinc-500 hover:text-zinc-300"
                  }`}
                >
                  预览
                </button>
              </div>
            </div>
            <div className="border-b border-zinc-800/40 px-4 py-2">
              <input
                ref={titleInputRef}
                type="text" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="请输入标题"
                disabled={!currentArticleId}
                className="w-full bg-transparent text-sm font-medium text-zinc-200 placeholder-zinc-600 focus:outline-none disabled:opacity-40"
              />
            </div>
            {previewMode ? (
              <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden bg-zinc-950 px-5 py-4">
                {markdown.trim() ? (
                  <div className="prose prose-invert max-w-none prose-headings:font-semibold prose-pre:bg-zinc-900 prose-pre:text-zinc-100 prose-a:text-emerald-400">
                    <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[rehypeHighlight]}>
                      {markdown}
                    </ReactMarkdown>
                  </div>
                ) : (
                  <div className="flex h-full items-center justify-center text-xs text-zinc-600">
                    暂无可预览的内容，切换到「编辑」开始写作
                  </div>
                )}
              </div>
            ) : (
              <textarea
                className="min-h-0 flex-1 resize-none bg-zinc-950 p-4 font-mono text-sm leading-relaxed text-zinc-200 placeholder-zinc-600 focus:outline-none disabled:opacity-40"
                placeholder={currentArticleId ? "开始创作你的 Markdown…" : "请先新建或选择一篇文章"}
                value={markdown} onChange={(e) => setMarkdown(e.target.value)}
                disabled={!currentArticleId}
              />
            )}
          </div>
        </Panel>

        <Separator className="group relative flex w-1.5 items-center justify-center bg-zinc-900 transition-colors hover:bg-emerald-500/20 data-[resize-handle-active]:bg-emerald-500/40">
          <div className="h-8 w-0.5 rounded-full bg-zinc-700 transition-colors group-hover:bg-emerald-500/60 group-data-[resize-handle-active]:bg-emerald-500" />
        </Separator>

        <Panel defaultSize="35%" minSize={280} maxSize="50%">
          <div className="flex h-full min-h-0 min-w-[280px] flex-col overflow-hidden">
            <div className="border-b border-zinc-800/70 px-4 py-2 text-[11px] font-medium uppercase tracking-wider text-zinc-500">
              平台选择
            </div>
            <div className="min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-3 py-2">
              {PLATFORMS.map((p) => {
                const locked = !p.available;
                return (
                <div key={p.id} className={`group flex items-center justify-between rounded-lg px-3 py-2.5 transition-colors min-w-0 ${locked ? "opacity-60" : "hover:bg-zinc-800/50"}`}>
                  <label className={`flex items-center gap-2.5 ${locked ? "cursor-not-allowed" : "cursor-pointer"}`}>
                    <input
                      type="checkbox" checked={selectedPlatforms.has(p.id)} onChange={() => togglePlatform(p.id)}
                      disabled={locked}
                      className="h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-800 text-emerald-500 focus:ring-emerald-500/30 disabled:cursor-not-allowed"
                    />
                    <span className="min-w-0 truncate text-sm text-zinc-200">{p.name}</span>
                  </label>
                  <div className="flex shrink-0 items-center gap-2">
                    <span className={`shrink-0 whitespace-nowrap text-[11px] font-medium ${!locked && loginStatus[p.id] ? "text-emerald-400" : "text-zinc-500"}`}>
                      {runningPlatforms.includes(p.id) ? "同步中" : locked ? "开发中" : loginStatus[p.id] ?? "未配置"}
                    </span>
                    <button
                      onClick={(e) => handlePlatformConfigClick(e, p.id)}
                      disabled={locked}
                      title={locked ? `${p.name} 的适配器开发中` : undefined}
                      className="shrink-0 whitespace-nowrap rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-400 transition-colors hover:border-amber-500/50 hover:bg-amber-500/20 disabled:cursor-not-allowed disabled:border-zinc-700 disabled:bg-zinc-800/40 disabled:text-zinc-500 disabled:hover:border-zinc-700 disabled:hover:bg-zinc-800/40"
                    >
                      {locked ? "开发中" : p.configType === "webview" ? "登录" : "配置"}
                    </button>
                  </div>
                </div>
                );
              })}
            </div>
            <div className="border-t border-zinc-800/70 p-3 space-y-2">
              <label className="flex cursor-pointer items-start gap-2 rounded-lg bg-zinc-900/60 px-3 py-2">
                <input
                  type="checkbox"
                  checked={publishOverwrite}
                  onChange={(e) => setPublishOverwrite(e.target.checked)}
                  className="mt-0.5 h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-800 text-red-500 focus:ring-red-500/30"
                />
                <span className="text-[11px] leading-relaxed text-zinc-300">
                  覆盖已发布的文章
                  <span className="block text-zinc-500">不勾选时仅保存草稿；勾选后将直接改动线上文章内容</span>
                </span>
              </label>
              {publishOverwrite && overwriteHints.length > 0 && (
                <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                  {overwriteHints.map((text, index) => (
                    <span key={index} className="block">
                      {text}
                    </span>
                  ))}
                </div>
              )}
              <button
                onClick={handlePublishAll}
                disabled={publishing || selectedPlatforms.size === 0 || !hasContent}
                className={`w-full whitespace-nowrap rounded-lg bg-emerald-600 py-2.5 text-sm font-semibold text-white transition-all hover:bg-emerald-500 disabled:cursor-not-allowed ${publishing || selectedPlatforms.size === 0 || !hasContent ? "opacity-30" : ""}`}
              >
                {publishing
                  ? "同步中..."
                  : !hasContent
                    ? "请先输入文章内容"
                    : selectedPlatforms.size === 0
                      ? "请先勾选平台"
                    : (() => {
                        const hasMapping = [...selectedPlatforms].some((id) => findUsableSyncEntry(syncMap, id) !== undefined);
                        const prefix = hasMapping ? "同步并更新线上文章" : publishOverwrite ? "一键同步并发布" : "一键同步草稿";
                        return `${prefix} (${selectedPlatforms.size})`;
                      })()}
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

      {/* ── Cookie Config Modal（掘金/CSDN 走一键登录；微信公众号 requiredConfigKeys 非空，走手动 Cookie+token） ── */}
      {configPlatform && currentPlatform?.configType === "cookie" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setConfigPlatform(null)}>
          <div className="w-[440px] rounded-xl border border-zinc-800 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-zinc-100">{currentPlatform.name} 登录</h2>
            <p className="mt-1 text-xs text-zinc-500">
              {manualOnlyPlatform
                ? "手动填写 Cookie 与 token，两者缺一不可（扫码登录拿不到 token，故不提供一键登录）"
                : "一键登录后自动获取 Cookie 并加密保存，无需手动复制"}
            </p>
            {manualOnlyPlatform ? (
              <div className="mt-5 space-y-4">
                <div>
                  <label className="mb-1.5 block text-xs font-medium text-zinc-300">Cookie</label>
                  <textarea
                    className={TEXTAREA_CLS + " h-28"}
                    placeholder={`从浏览器开发者工具中复制 ${currentPlatform.name} 的完整 Cookie 字符串`}
                    value={cookieConfigs[configPlatform] ?? ""}
                    onChange={(e) => setCookieConfigs((prev) => ({ ...prev, [configPlatform!]: e.target.value }))}
                  />
                  <p className="mt-1.5 text-[11px] leading-relaxed text-zinc-500">
                    F12 → Network → 任意请求 → Headers → Cookie，复制完整字符串（不是单个字段的值）。
                  </p>
                </div>
                <div>
                  <label className="mb-1.5 block text-xs font-medium text-zinc-300">token</label>
                  <input
                    type="text"
                    className={INPUT_CLS}
                    placeholder="公众号后台地址栏中 token= 后面的值"
                    value={configTokens[configPlatform] ?? ""}
                    onChange={(e) => setConfigTokens((prev) => ({ ...prev, [configPlatform!]: e.target.value }))}
                  />
                  <p className="mt-1.5 text-[11px] leading-relaxed text-zinc-500">
                    登录后台后地址栏形如 ...&amp;token=<span className="text-emerald-400/70">1234567890</span>&amp;lang=zh_CN，取 token= 与下一个 &amp; 之间的数字。token 不在 Cookie 里，重新登录会变。
                  </p>
                </div>
                <button
                  onClick={(e) => handleOpenUrl(e, currentPlatform.loginUrl, currentPlatform.name)}
                  className="inline-flex items-center gap-1.5 text-[11px] font-medium text-emerald-400 transition-colors hover:text-emerald-300"
                >
                  <LinkIcon /> 前往{currentPlatform.name}后台
                </button>
                <button
                  onClick={() => handleSaveCookieConfig(configPlatform!)}
                  className="w-full rounded-md bg-emerald-600 py-2 text-sm font-medium text-white transition-colors hover:bg-emerald-500"
                >
                  保存 Cookie 与 token
                </button>
              </div>
            ) : (
            <>
            <div className="mt-5">
              <button
                onClick={(e) => handleOpenLogin(e, configPlatform!)}
                className="w-full rounded-lg border border-emerald-500/30 bg-emerald-500/10 py-3 text-sm font-medium text-emerald-400 transition-colors hover:border-emerald-500/50 hover:bg-emerald-500/20"
              >
                打开{currentPlatform.name}扫码登录
              </button>
              <p className="mt-2 text-center text-[11px] text-zinc-500">
                登录成功后窗口会自动关闭，状态变为「已登录」
              </p>
            </div>
            <div className="mt-5 border-t border-zinc-800 pt-4">
              <button
                onClick={() => setManualCookieOpen((prev) => !prev)}
                className="inline-flex items-center gap-1.5 text-[11px] font-medium text-zinc-400 transition-colors hover:text-zinc-200"
              >
                <svg
                  className={`h-3 w-3 transition-transform ${manualCookieOpen ? "rotate-90" : ""}`}
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}
                >
                  <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
                </svg>
                手动粘贴 Cookie（备用方式）
              </button>
              {manualCookieOpen && (
                <div className="mt-3">
                  <textarea
                    className={TEXTAREA_CLS + " h-28"}
                    placeholder={`从浏览器开发者工具中复制 ${currentPlatform.name} 的完整 Cookie 字符串`}
                    value={cookieConfigs[configPlatform] ?? ""}
                    onChange={(e) => setCookieConfigs((prev) => ({ ...prev, [configPlatform!]: e.target.value }))}
                  />
                  <div className="mt-2 rounded-lg bg-zinc-900 px-3 py-2.5">
                    <p className="text-[11px] leading-relaxed text-zinc-400">
                      <span className="font-medium text-zinc-300">如何获取？</span>{" "}
                      在浏览器中登录{currentPlatform.name}，按 F12 打开开发者工具 → Network → 任意请求 → Headers → Cookie，复制完整值粘贴到上方。
                    </p>
                    <button
                      onClick={(e) => handleOpenUrl(e, currentPlatform.loginUrl, currentPlatform.name)}
                      className="mt-2 inline-flex items-center gap-1.5 rounded-md text-[11px] font-medium text-emerald-400 transition-colors hover:text-emerald-300"
                    >
                      <LinkIcon /> 前往{currentPlatform.name}登录
                    </button>
                  </div>
                  <button
                    onClick={() => handleSaveCookieConfig(configPlatform!)}
                    className="mt-3 w-full rounded-md bg-zinc-800 py-2 text-sm font-medium text-zinc-100 transition-colors hover:bg-zinc-700"
                  >
                    保存粘贴的 Cookie
                  </button>
                </div>
              )}
            </div>
            </>
            )}
            <div className="mt-6 flex items-center justify-end gap-3">
              <button onClick={() => setConfigPlatform(null)} className="rounded-md px-4 py-2 text-sm font-medium text-zinc-400 transition-colors hover:text-zinc-100">关闭</button>
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
                onClick={(e) => handleOpenLogin(e, configPlatform!)}
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

      {/* ── Overwrite Confirm Modal ── */}
      {pendingConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setPendingConfirm(null)}>
          <div className="w-[440px] rounded-xl border border-red-500/30 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-red-400">确认覆盖线上文章？</h2>
            <p className="mt-2 text-xs leading-relaxed text-zinc-400">
              注意：这将会直接覆盖线上文章，且不可逆，是否继续？
            </p>
            <div className="mt-3 rounded-lg bg-zinc-900 px-3 py-2 text-[11px] leading-relaxed text-zinc-400">
              即将覆盖：
              <span className="text-zinc-200">
                {pendingConfirm.platforms.map((id) => PLATFORMS.find((p) => p.id === id)?.name ?? id).join("、")}
              </span>
              {pendingConfirm.platforms.includes("juejin") && (
                <span className="mt-1 block text-amber-300">
                  掘金目前仅支持保存草稿，请到掘金后台手动发布。
                </span>
              )}
            </div>
            <div className="mt-6 flex items-center justify-end gap-3">
              <button
                onClick={() => setPendingConfirm(null)}
                className="rounded-md px-4 py-2 text-sm font-medium text-zinc-400 transition-colors hover:text-zinc-100"
              >
                取消
              </button>
              <button
                onClick={handleConfirmOverwrite}
                className="rounded-md bg-red-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-red-500"
              >
                确认覆盖
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ── Unconfigured Platforms Warning Modal ── */}
      {pendingConfigWarn && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm" onClick={() => setPendingConfigWarn(null)}>
          <div className="w-[400px] rounded-xl border border-amber-500/30 bg-zinc-950 p-6 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h2 className="text-base font-semibold text-amber-400">以下平台尚未配置</h2>
            <p className="mt-2 text-xs leading-relaxed text-zinc-400">
              请先完成配置，或取消勾选未配置的平台后再同步。
            </p>
            <div className="mt-3 rounded-lg bg-amber-500/[0.08] px-3 py-2.5">
              {pendingConfigWarn.names.map((name, i) => (
                <div key={i} className="flex items-center justify-between text-[11px] leading-relaxed">
                  <span className="text-zinc-200">{name}</span>
                  <span className="text-amber-400/70">未配置</span>
                </div>
              ))}
            </div>
            <div className="mt-5 text-[11px] leading-relaxed text-zinc-500">
              提示：在右侧平台列表中，点击「配置」按钮填入凭据即可完成配置。
            </div>
            <div className="mt-4 flex justify-end">
              <button
                onClick={() => setPendingConfigWarn(null)}
                className="rounded-md bg-amber-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-amber-500"
              >
                我知道了
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ── Log Panel ── */}
      <div className="flex h-40 flex-col border-t border-zinc-800/70">
        <div className="flex items-center justify-between border-b border-zinc-800/70 px-4 py-1.5">
          <span className="text-[11px] font-medium uppercase tracking-wider text-zinc-500">
            同步日志{logs.length > 0 ? ` (${logs.length})` : ""}
          </span>
          <button
            onClick={handleExportLogs}
            disabled={logs.length === 0 || exportingLogs}
            title={logs.length === 0 ? "暂无日志可导出" : "导出为 JSONL 文件"}
            className={`rounded-md border border-zinc-700 px-2 py-0.5 text-[11px] font-medium text-zinc-400 transition-colors hover:border-emerald-500/50 hover:text-emerald-400 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:border-zinc-700 disabled:hover:text-zinc-400`}
          >
            {exportingLogs ? "导出中..." : "导出日志"}
          </button>
        </div>
        <div className="flex-1 overflow-y-auto bg-zinc-950 px-4 py-1.5 font-mono text-xs">
          {logs.length === 0 ? (
            <span className="text-zinc-600">暂无日志。勾选平台后点击「一键同步」开始。</span>
          ) : (
            <>
              {logs.map((log, i) => (
                <div key={i} className="py-0.5">
                  <span className="text-zinc-600">[{log.timestamp}]</span>{" "}
                  <span className="text-emerald-400/80">{log.platformName}</span>{" "}
                  {log.operation === "update" && (
                    <span className="text-sky-400/70">[更新] </span>
                  )}
                  {log.operation === "create" && (
                    <span className="text-zinc-500">[新建] </span>
                  )}
                  {log.isPublished && (
                    <span className="text-red-400/70">[发布] </span>
                  )}
                  <span
                    className={
                      log.outcome === "success"
                        ? "text-emerald-400"
                        : log.outcome === "running"
                          ? "text-amber-400"
                          : "text-red-400"
                    }
                  >
                    {log.message}
                  </span>
                  {log.draftUrl && (
                    <>
                      {" "}
                      <button
                        onClick={() => handleOpenEditUrl(log.draftUrl!)}
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

      {/* ── Settings Modal ── */}
      {settingsOpen && (
        <SettingsPanel
          articleCount={articles.length}
          onClose={() => setSettingsOpen(false)}
          onToast={showToast}
          onDataChanged={handleDataChanged}
        />
      )}

      {/* ── About / Support Modal ── */}
      {aboutOpen && (
        <AboutDialog onClose={() => setAboutOpen(false)} onToast={showToast} />
      )}

      {/* ── Toast ── */}
      <Toast message={toast} onDismiss={dismissToast} />
    </div>
  );
}

export default App;