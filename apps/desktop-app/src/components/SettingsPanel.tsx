import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { PLATFORM_REGISTRY } from "@openpub/core";

interface ImportReport {
  platforms: number;
  articles: number;
  syncMaps: number;
}

interface ResetReport {
  configKeys: number;
  credentialKeys: number;
}

interface ExportFileInfo {
  path: string;
  fileName: string;
  modifiedAt: number;
  sizeBytes: number;
}

interface UpdateCheck {
  state: "upToDate" | "updateAvailable" | "failed";
  current: string;
  latest: string | null;
  hasUpdate: boolean;
  downloadUrl: string | null;
  notes: string | null;
  message: string;
}

type ConfirmKind = "export" | "import" | "reset" | "cleanup";

interface SettingsPanelProps {
  articleCount: number;
  onClose: () => void;
  onToast: (text: string, tone?: "success" | "error") => void;
  onDataChanged: () => void;
}

const PLATFORM_IDS = PLATFORM_REGISTRY.map((p) => p.id);

type SectionId = "backup" | "restore" | "update" | "danger";

const NAV_ITEMS: { id: SectionId; label: string }[] = [
  { id: "backup", label: "配置备份" },
  { id: "restore", label: "从备份恢复" },
  { id: "update", label: "检查更新" },
  { id: "danger", label: "一键清空" },
];

const SECTION_CLS = "rounded-xl border border-zinc-800/80 bg-zinc-900/40 p-4";
const BTN_PRIMARY =
  "inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-1.5 text-xs font-medium text-emerald-400 transition-colors hover:border-emerald-500/50 hover:bg-emerald-500/20 disabled:cursor-not-allowed disabled:opacity-40";
const BTN_DANGER =
  "inline-flex items-center gap-1.5 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-1.5 text-xs font-medium text-red-400 transition-colors hover:border-red-500/50 hover:bg-red-500/20 disabled:cursor-not-allowed disabled:opacity-40";
const BTN_GHOST =
  "inline-flex items-center gap-1.5 rounded-lg border border-zinc-700 px-3 py-1.5 text-xs font-medium text-zinc-400 transition-colors hover:border-zinc-600 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-40";

function formatTime(seconds: number): string {
  if (!seconds) return "未知时间";
  const d = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export function SettingsPanel({ articleCount, onClose, onToast, onDataChanged }: SettingsPanelProps) {
  const [exports, setExports] = useState<ExportFileInfo[]>([]);
  const [section, setSection] = useState<SectionId>("backup");
  const [lastExportPath, setLastExportPath] = useState<string | null>(null);
  const [selectedSource, setSelectedSource] = useState<string>("");
  const [manualPath, setManualPath] = useState("");
  const [confirmKind, setConfirmKind] = useState<ConfirmKind | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState<"export" | "import" | "reset" | "cleanup" | null>(null);
  const [appVersion, setAppVersion] = useState("");
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  const sourcePath = manualPath.trim() || selectedSource;

  const refreshExports = () => {
    invoke<ExportFileInfo[]>("list_export_files")
      .then((list) => setExports(list ?? []))
      .catch(() => setExports([]));
  };

  useEffect(() => {
    refreshExports();
  }, []);

  useEffect(() => {
    invoke<{ appVersion: string }>("get_host_info")
      .then((info) => setAppVersion(info.appVersion))
      .catch(() => setAppVersion(""));
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    setAcknowledged(false);
  }, [confirmKind]);

  const openConfirm = (kind: ConfirmKind) => {
    if (kind === "import" && !sourcePath) {
      onToast("请先选择或填写配置文件路径", "error");
      return;
    }
    setConfirmKind(kind);
    setAcknowledged(false);
  };

  const handleExport = async () => {
    setBusy("export");
    try {
      const path = await invoke<string>("export_config", { targetPath: null });
      setLastExportPath(path);
      refreshExports();
      onToast("配置已导出");
    } catch (err) {
      onToast(`导出失败: ${err}`, "error");
    } finally {
      setBusy(null);
      setConfirmKind(null);
    }
  };

  const handleImport = async () => {
    setBusy("import");
    try {
      const report = await invoke<ImportReport>("import_config", { sourcePath });
      onDataChanged();
      onToast(
        `导入成功：${report.platforms} 个平台配置、${report.articles} 篇文章、${report.syncMaps} 条同步记录`,
      );
    } catch (err) {
      onToast(`导入失败，已还原原有数据: ${err}`, "error");
    } finally {
      setBusy(null);
      setConfirmKind(null);
    }
  };

  const handleReset = async () => {
    setBusy("reset");
    try {
      const report = await invoke<ResetReport>("reset_all_data", { platformIds: PLATFORM_IDS });
      onDataChanged();
      onToast(`已清空 ${report.configKeys + report.credentialKeys} 项数据，界面偏好已保留`);
    } catch (err) {
      onToast(`清空失败: ${err}`, "error");
    } finally {
      setBusy(null);
      setConfirmKind(null);
    }
  };

  const handleCleanup = async () => {
    setBusy("cleanup");
    try {
      const report = await invoke<{ removedCredentialKeys: number }>(
        "cleanup_invalid_credentials_cmd",
      );
      onDataChanged();
      onToast(`已清理 ${report.removedCredentialKeys} 项无效凭据`);
    } catch (err) {
      onToast(`清理失败: ${err}`, "error");
    } finally {
      setBusy(null);
      setConfirmKind(null);
    }
  };

  const handleReveal = async (path: string) => {
    try {
      await revealItemInDir(path);
    } catch (err) {
      onToast(`打开所在文件夹失败: ${err}`, "error");
    }
  };

  const handleCheckUpdate = async () => {
    if (checkingUpdate) return;
    setCheckingUpdate(true);
    try {
      const result = await invoke<UpdateCheck>("check_update");
      setUpdateInfo(result);
      if (result.state === "updateAvailable") {
        onToast(`发现新版本 ${result.latest}`);
      } else if (result.state === "failed") {
        onToast(result.message, "error");
      } else {
        onToast("当前已是最新版本");
      }
    } catch (err) {
      setUpdateInfo({
        state: "failed",
        current: appVersion,
        latest: null,
        hasUpdate: false,
        downloadUrl: null,
        notes: null,
        message: `检查更新失败: ${err}`,
      });
      onToast(`检查更新失败: ${err}`, "error");
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleOpenDownload = async () => {
    if (!updateInfo?.downloadUrl) return;
    try {
      await openUrl(updateInfo.downloadUrl);
    } catch (err) {
      onToast(`打开下载页失败: ${err}`, "error");
    }
  };

  const confirmTitle = useMemo(() => {
    if (confirmKind === "export") return "导出配置";
    if (confirmKind === "import") return "导入配置";
    if (confirmKind === "cleanup") return "清理无效凭据";
    return "一键清空";
  }, [confirmKind]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-6 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex max-h-[85vh] w-full max-w-[620px] flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-zinc-950 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-zinc-800/80 px-6 py-4">
          <div className="flex items-center gap-2.5">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-zinc-800 text-zinc-300">
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37a1.724 1.724 0 002.572-1.065z" />
                <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </span>
            <h2 className="text-base font-semibold tracking-tight text-zinc-100">设置中心</h2>
          </div>
          <button
            onClick={onClose}
            aria-label="关闭"
            className="rounded-md p-1.5 text-zinc-500 transition-colors hover:bg-zinc-800/60 hover:text-zinc-200"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          <nav className="w-[112px] shrink-0 space-y-1 border-r border-zinc-800/80 px-2.5 py-4">
            {NAV_ITEMS.map((item) => (
              <button
                key={item.id}
                onClick={() => setSection(item.id)}
                className={`w-full rounded-lg px-2.5 py-2 text-left text-[11px] font-medium transition-colors ${
                  section === item.id
                    ? item.id === "danger"
                      ? "bg-red-500/10 text-red-300"
                      : "bg-emerald-500/10 text-emerald-400"
                    : "text-zinc-500 hover:bg-zinc-800/50 hover:text-zinc-300"
                }`}
              >
                {item.label}
              </button>
            ))}
          </nav>

          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
          {section === "backup" && (
          <section className={SECTION_CLS}>
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-medium text-zinc-100">配置备份</h3>
                <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
                  把文章、同步记录与各平台凭据打包成一个 JSON 文件，方便换机迁移或留档。
                </p>
              </div>
              <button
                onClick={() => openConfirm("export")}
                disabled={busy !== null}
                className={BTN_PRIMARY}
              >
                <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3" />
                </svg>
                导出配置
              </button>
            </div>
            {lastExportPath && (
              <div className="mt-3 rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2">
                <p className="break-all text-[11px] text-zinc-400">{lastExportPath}</p>
                <button
                  onClick={() => handleReveal(lastExportPath)}
                  className="mt-1.5 text-[11px] text-emerald-400/80 underline decoration-emerald-400/30 underline-offset-2 transition-colors hover:text-emerald-300"
                >
                  打开所在文件夹
                </button>
              </div>
            )}
          </section>
          )}

          {section === "restore" && (
          <section className={SECTION_CLS}>
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-medium text-zinc-100">从备份恢复</h3>
                <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
                  导入会覆盖当前全部数据。写入失败时自动回滚到导入前的状态，不会留下半份数据。
                </p>
              </div>
              <button
                onClick={() => openConfirm("import")}
                disabled={busy !== null || !sourcePath}
                className={BTN_GHOST}
              >
                <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 8l5-5 5 5M12 3v12" />
                </svg>
                导入配置
              </button>
            </div>
            <div className="mt-3 space-y-1.5">
              {exports.length === 0 ? (
                <p className="text-[11px] text-zinc-600">还没有导出过配置文件。</p>
              ) : (
                exports.map((file) => {
                  const active = selectedSource === file.path && !manualPath.trim();
                  return (
                    <div
                      key={file.path}
                      onClick={() => {
                        setSelectedSource(file.path);
                        setManualPath("");
                      }}
                      className={`cursor-pointer rounded-lg border px-3 py-2 transition-colors ${
                        active
                          ? "border-emerald-500/40 bg-emerald-500/10"
                          : "border-zinc-800 bg-zinc-950/40 hover:border-zinc-700"
                      }`}
                    >
                      <p className={`truncate text-[11px] ${active ? "text-emerald-400" : "text-zinc-300"}`}>
                        {file.fileName}
                      </p>
                      <p className="mt-0.5 text-[10px] text-zinc-600">
                        {formatTime(file.modifiedAt)} · {formatSize(file.sizeBytes)}
                      </p>
                    </div>
                  );
                })
              )}
            </div>
            <input
              value={manualPath}
              onChange={(e) => setManualPath(e.target.value)}
              placeholder="或粘贴其他机器的配置文件绝对路径"
              className="mt-2 w-full rounded-md border border-zinc-700 bg-zinc-900 px-2.5 py-1.5 text-[11px] text-zinc-200 placeholder-zinc-600 focus:border-emerald-500 focus:outline-none"
            />
          </section>
          )}

          {section === "update" && (
          <section className={SECTION_CLS}>
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-medium text-zinc-100">版本更新</h3>
                <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
                  手动检查 Gitee 上的最新版本。应用启动时不会自动联网检查。
                </p>
              </div>
              <button
                onClick={handleCheckUpdate}
                disabled={checkingUpdate}
                className={BTN_PRIMARY}
              >
                <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </svg>
                {checkingUpdate ? "检查中…" : "检查更新"}
              </button>
            </div>

            <div className="mt-3 space-y-2">
              <div className="flex items-center justify-between rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2">
                <span className="text-[11px] text-zinc-500">当前版本</span>
                <span className="font-mono text-[11px] text-zinc-300">
                  {updateInfo?.current || appVersion || "—"}
                </span>
              </div>
              <div className="flex items-center justify-between rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2">
                <span className="text-[11px] text-zinc-500">远端版本</span>
                <span className="font-mono text-[11px] text-zinc-300">
                  {updateInfo?.latest ?? "—"}
                </span>
              </div>
            </div>

            {updateInfo?.state === "updateAvailable" && (
              <div className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/[0.06] px-3 py-2.5">
                <p className="text-[11px] font-medium text-emerald-400">
                  发现新版本 {updateInfo.latest}
                </p>
                {updateInfo.notes && (
                  <p className="mt-1.5 whitespace-pre-wrap text-[11px] leading-relaxed text-zinc-400">
                    {updateInfo.notes}
                  </p>
                )}
                <button onClick={handleOpenDownload} className={`mt-2.5 ${BTN_PRIMARY}`}>
                  <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3" />
                  </svg>
                  前往下载
                </button>
              </div>
            )}

            {updateInfo?.state === "upToDate" && (
              <p className="mt-3 rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2 text-[11px] text-zinc-400">
                当前已是最新版本。
              </p>
            )}

            {updateInfo?.state === "failed" && (
              <p className="mt-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                {updateInfo.message}
              </p>
            )}
          </section>
          )}

          {section === "danger" && (
          <>
            <section className="rounded-xl border border-red-500/25 bg-red-500/[0.04] p-4">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h3 className="text-sm font-medium text-red-300">一键清空</h3>
                  <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
                    清除全部文章（当前 {articleCount} 篇）、同步记录、平台配置与已保存的登录凭据。
                    主题、布局等界面偏好会保留。
                  </p>
                </div>
                <button
                  onClick={() => openConfirm("reset")}
                  disabled={busy !== null}
                  className={BTN_DANGER}
                >
                  <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                  清空数据
                </button>
              </div>
            </section>

            <section className="mt-3 rounded-xl border border-zinc-800/80 bg-zinc-900/40 p-4">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h3 className="text-sm font-medium text-zinc-100">清理无效凭据</h3>
                  <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
                    只清理非 Cookie 类平台的残留凭据（如早期版本误写的博客园脏数据），不影响掘金等正常登录态。
                  </p>
                </div>
                <button
                  onClick={() => openConfirm("cleanup")}
                  disabled={busy !== null}
                  className={BTN_DANGER}
                >
                  <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M20 13V11a8 8 0 10-4 6.928M12 7v4l3 2m-3-7h.01" />
                  </svg>
                  清理
                </button>
              </div>
            </section>
          </>
          )}
          </div>
        </div>
      </div>
      {confirmKind && (
        <div
          className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-6"
          onClick={(e) => {
            e.stopPropagation();
            if (busy === null) setConfirmKind(null);
          }}
        >
          <div
            className="w-full max-w-[420px] overflow-hidden rounded-xl border border-zinc-700 bg-zinc-900 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="border-b border-zinc-800 px-5 py-3.5">
              <h3 className="text-sm font-semibold text-zinc-100">{confirmTitle}</h3>
            </div>
            <div className="space-y-3 px-5 py-4">
              {confirmKind === "export" && (
                <>
                  <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                    文件含明文凭据：导出的 JSON 里各平台 Cookie、API Token 均为明文，请妥善保管，不要上传到公开仓库或网盘分享。
                  </p>
                  <p className="text-[11px] leading-relaxed text-zinc-400">
                    导出内容包含全部文章、同步记录与平台配置，可用于换机迁移。
                  </p>
                </>
              )}
              {confirmKind === "import" && (
                <>
                  <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                    导入会覆盖当前全部数据，且无法撤销。如担心丢失，请先导出一份当前配置。
                  </p>
                  <p className="break-all rounded-lg border border-zinc-800 bg-zinc-950/60 px-3 py-2 text-[11px] text-zinc-400">
                    {sourcePath}
                  </p>
                </>
              )}
              {confirmKind === "reset" && (
                <>
                  <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-[11px] leading-relaxed text-red-300">
                    将永久删除全部文章、同步记录、平台配置与登录凭据，此操作无法撤销。
                  </p>
                  <label className="flex cursor-pointer items-start gap-2 text-[11px] text-zinc-400">
                    <input
                      type="checkbox"
                      checked={acknowledged}
                      onChange={(e) => setAcknowledged(e.target.checked)}
                      className="mt-0.5 h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-900 accent-red-500"
                    />
                    我已知晓数据无法恢复，确认清空
                  </label>
                </>
              )}
              {confirmKind === "cleanup" && (
                <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-300">
                  只清理非 Cookie 类平台的残留凭据，不影响掘金等正常登录态。
                </p>
              )}
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-zinc-800 px-5 py-3">
              <button
                onClick={() => setConfirmKind(null)}
                disabled={busy !== null}
                className={BTN_GHOST}
              >
                取消
              </button>
              <button
                onClick={
                  confirmKind === "export"
                    ? handleExport
                    : confirmKind === "import"
                      ? handleImport
                      : confirmKind === "cleanup"
                        ? handleCleanup
                        : handleReset
                }
                disabled={
                  busy !== null || (confirmKind === "reset" && !acknowledged)
                }
                className={
                  confirmKind === "reset" || confirmKind === "cleanup"
                    ? BTN_DANGER
                    : BTN_PRIMARY
                }
              >
                {busy !== null
                  ? "处理中…"
                  : confirmKind === "export"
                    ? "仍要导出"
                    : confirmKind === "import"
                      ? "确认导入"
                      : confirmKind === "cleanup"
                        ? "确认清理"
                        : "确认清空"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}