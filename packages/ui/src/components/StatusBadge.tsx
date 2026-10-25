import React from "react";

export type StatusType = "success" | "error" | "warning" | "info" | "loading";

export interface StatusBadgeProps {
  status: StatusType;
  text?: string;
}

const statusStyles: Record<StatusType, string> = {
  success:
    "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200",
  error: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200",
  warning:
    "bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200",
  info: "bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200",
  loading:
    "bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-200",
};

const defaultText: Record<StatusType, string> = {
  success: "Success",
  error: "Error",
  warning: "Warning",
  info: "Info",
  loading: "Loading",
};

export const StatusBadge: React.FC<StatusBadgeProps> = ({ status, text }) => {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${statusStyles[status]}`}
    >
      {status === "loading" && (
        <span className="h-2 w-2 animate-spin rounded-full border border-current border-t-transparent" />
      )}
      {text ?? defaultText[status]}
    </span>
  );
};