import { useCallback, useEffect, useState } from "react";

export interface ToastMessage {
  id: number;
  text: string;
  tone: "success" | "error";
}

const AUTO_DISMISS_MS = 2500;

export function useToast() {
  const [toast, setToast] = useState<ToastMessage | null>(null);

  const showToast = useCallback((text: string, tone: ToastMessage["tone"] = "success") => {
    setToast({ id: Date.now(), text, tone });
  }, []);

  const dismissToast = useCallback(() => setToast(null), []);

  return { toast, showToast, dismissToast };
}

export function Toast({
  message,
  onDismiss,
}: {
  message: ToastMessage | null;
  onDismiss: () => void;
}) {
  useEffect(() => {
    if (!message) return;
    const timer = window.setTimeout(onDismiss, AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [message, onDismiss]);

  if (!message) return null;

  const isError = message.tone === "error";

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-8 z-[60] flex justify-center">
      <div
        key={message.id}
        className={`flex items-center gap-2 rounded-lg border px-4 py-2.5 text-xs font-medium shadow-2xl backdrop-blur-md ${
          isError
            ? "border-red-500/40 bg-red-500/15 text-red-300"
            : "border-emerald-500/40 bg-emerald-500/15 text-emerald-300"
        }`}
      >
        <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.2}>
          {isError ? (
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
          ) : (
            <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
          )}
        </svg>
        {message.text}
      </div>
    </div>
  );
}