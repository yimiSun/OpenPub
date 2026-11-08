import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { APP_SLOGAN } from "@openpub/core";

const GITEE_URL = "https://gitee.com/tianyeHome/OpenPub";
const GLM_URL =
  "https://www.bigmodel.cn/invite?icode=ejw1KvHtq349R9J2498tZEjPr3uHog9F4g5tjuOUqno%3D";
const ALIYUN_URL = "https://www.aliyun.com/minisite/goods?userCode=4ls1opon";
const HIDE_PROMO_KEY = "openpub_hide_promo";

interface HostInfo {
  appVersion: string;
  osVersion: string;
  engine: string;
}

interface AboutDialogProps {
  onClose: () => void;
  onToast: (text: string, tone?: "success" | "error") => void;
}

const QR_ITEMS = [
  { key: "alipay", label: "支付宝", src: "/images/alipay_qr.png" },
  { key: "wechat", label: "微信", src: "/images/wechat_qr.png" },
] as const;

const PROMO_ITEMS = [
  {
    id: "glm",
    title: "智谱 GLM 免费体验",
    subtitle: "领 2000 万 Tokens",
    url: GLM_URL,
    accent: "emerald",
  },
  {
    id: "aliyun",
    title: "阿里云服务器特惠",
    subtitle: "新老用户专属折扣",
    url: ALIYUN_URL,
    accent: "sky",
  },
] as const;

export function AboutDialog({ onClose, onToast }: AboutDialogProps) {
  const [hostInfo, setHostInfo] = useState<HostInfo | null>(null);
  const [hidePromo, setHidePromo] = useState(
    () => localStorage.getItem(HIDE_PROMO_KEY) === "1",
  );
  const [failedImages, setFailedImages] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let cancelled = false;
    invoke<HostInfo>("get_host_info")
      .then((info) => {
        if (!cancelled) setHostInfo(info);
      })
      .catch(() => {
        if (!cancelled) setHostInfo(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const handleOpenExternal = async (url: string) => {
    try {
      await openUrl(url);
    } catch (err) {
      onToast(`打开浏览器失败: ${err}`, "error");
    }
  };

  const handleCopy = async (url: string) => {
    try {
      await writeText(url);
      onToast("已复制到剪贴板");
    } catch (err) {
      onToast(`复制失败: ${err}`, "error");
    }
  };

  const handleTogglePromo = (checked: boolean) => {
    setHidePromo(checked);
    localStorage.setItem(HIDE_PROMO_KEY, checked ? "1" : "0");
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-6 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex max-h-[85vh] w-full max-w-[560px] flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-zinc-950 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-zinc-800/80 px-6 py-4">
          <div className="flex items-center gap-2.5">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-400">
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z" />
              </svg>
            </span>
            <h2 className="text-base font-semibold tracking-tight text-zinc-100">关于 OpenPub</h2>
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

        <div className="flex-1 overflow-y-auto px-6 py-5">
          <section>
            <p className="text-sm font-medium leading-relaxed text-emerald-400">
              {APP_SLOGAN}
            </p>
            <p className="mt-1.5 text-[11px] leading-relaxed text-zinc-500">
              文章与 Cookie 全部留在你的电脑上，不经任何中转服务器。
            </p>
            <button
              onClick={() => handleOpenExternal(GITEE_URL)}
              className="mt-3 inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3.5 py-2 text-xs font-medium text-emerald-400 transition-colors hover:border-emerald-500/50 hover:bg-emerald-500/20"
            >
              <svg className="h-3.5 w-3.5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 .5C5.73.5.5 5.73.5 12c0 5.08 3.29 9.39 7.86 10.91.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.54-3.88-1.54-.53-1.33-1.29-1.69-1.29-1.69-1.05-.72.08-.7.08-.7 1.16.08 1.78 1.2 1.78 1.2 1.04 1.78 2.72 1.27 3.38.95.1-.73.4-1.27.73-1.56-2.55-.29-5.24-1.28-5.24-5.68 0-1.26.45-2.29 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .96-.31 3.15 1.18a10.9 10.9 0 015.74 0c2.19-1.49 3.15-1.18 3.15-1.18.63 1.59.23 2.76.11 3.05.74.8 1.19 1.83 1.19 3.09 0 4.41-2.7 5.38-5.26 5.66.41.36.78 1.08.78 2.18v3.23c0 .31.21.67.8.56A10.52 10.52 0 0022.5 12C22.5 5.73 18.27.5 12 .5z" />
              </svg>
              打开 Gitee
            </button>
          </section>

          <div className="my-5 h-px bg-zinc-800/80" />

          <section>
            <h3 className="text-[11px] font-medium uppercase tracking-wider text-zinc-500">
              支持开发者
            </h3>
            <p className="mt-2 text-[11px] leading-relaxed text-zinc-400">
              OpenPub 永久免费且开源。如果这个工具帮你省下了每月几十元的订阅费，欢迎请开发者喝杯咖啡，支持后续的持续更新。
            </p>
            <div className="mt-3 flex items-start gap-3">
              {QR_ITEMS.map((item) => (
                <div
                  key={item.key}
                  className="flex flex-1 flex-col items-center rounded-xl border border-zinc-800 bg-zinc-900/40 px-3 py-3"
                >
                  {failedImages[item.key] ? (
                    <div className="flex h-[132px] w-[132px] items-center justify-center rounded-lg border border-dashed border-zinc-700 bg-zinc-900 px-3 text-center text-[11px] leading-relaxed text-zinc-600">
                      二维码待补充
                    </div>
                  ) : (
                    <img
                      src={item.src}
                      alt={`${item.label}打赏二维码`}
                      onError={() =>
                        setFailedImages((prev) => ({ ...prev, [item.key]: true }))
                      }
                      className="h-[132px] w-[132px] rounded-lg bg-white object-contain p-1.5"
                    />
                  )}
                  <span className="mt-2 text-[11px] text-zinc-400">{item.label}打赏</span>
                </div>
              ))}
            </div>
          </section>

          <div className="my-5 h-px bg-zinc-800/80" />

          <section>
            {hidePromo ? (
              <div className="flex items-center justify-between rounded-lg border border-zinc-800 bg-zinc-900/30 px-3.5 py-2.5">
                <span className="text-[11px] text-zinc-500">推广内容已隐藏</span>
                <button
                  onClick={() => handleTogglePromo(false)}
                  className="text-[11px] font-medium text-emerald-400/80 transition-colors hover:text-emerald-300"
                >
                  重新显示
                </button>
              </div>
            ) : (
              <>
                <div className="flex items-center justify-between">
                  <h3 className="text-[11px] font-medium uppercase tracking-wider text-zinc-500">
                    极客推荐
                  </h3>
                  <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-zinc-500 transition-colors hover:text-zinc-400">
                    <input
                      type="checkbox"
                      checked={hidePromo}
                      onChange={(e) => handleTogglePromo(e.target.checked)}
                      className="h-3 w-3 cursor-pointer accent-emerald-500"
                    />
                    不再显示
                  </label>
                </div>
                <div className="mt-2.5 space-y-2">
                  {PROMO_ITEMS.map((item) => (
                    <div
                      key={item.id}
                      className="rounded-xl border border-zinc-800 bg-zinc-900/40 px-3.5 py-3"
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="min-w-0">
                          <p
                            className={`truncate text-xs font-medium ${
                              item.accent === "emerald" ? "text-emerald-400" : "text-sky-400"
                            }`}
                          >
                            {item.title}
                          </p>
                          <p className="mt-0.5 truncate text-[11px] text-zinc-500">
                            {item.subtitle}
                          </p>
                        </div>
                        <div className="flex shrink-0 items-center gap-1.5">
                          <button
                            onClick={() => handleCopy(item.url)}
                            className="rounded-md border border-zinc-700 px-2.5 py-1 text-[11px] text-zinc-300 transition-colors hover:border-emerald-500/50 hover:text-emerald-400"
                          >
                            复制链接
                          </button>
                          <button
                            onClick={() => handleOpenExternal(item.url)}
                            className="rounded-md border border-zinc-700 px-2.5 py-1 text-[11px] text-zinc-300 transition-colors hover:border-emerald-500/50 hover:text-emerald-400"
                          >
                            浏览器打开
                          </button>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </>
            )}
          </section>
        </div>

        <div className="border-t border-zinc-800/80 px-6 py-3">
          <div className="flex flex-wrap items-center gap-x-5 gap-y-1 text-[11px] text-zinc-500">
            <span>
              版本 <span className="font-mono text-zinc-400">{hostInfo?.appVersion ?? "—"}</span>
            </span>
            <span>
              系统 <span className="text-zinc-400">{hostInfo?.osVersion ?? "—"}</span>
            </span>
            <span>
              引擎 <span className="text-zinc-400">{hostInfo?.engine ?? "Tauri v2 + Rust"}</span>
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}