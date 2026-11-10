export interface SyncEntry {
  id: string;
  published: boolean;
  bodyHash: string;
  /**
   * 草稿模式的远端草稿 ID（如小红书 `draftId`）。
   * 发布后 `id` 会变成正式文章 ID，但草稿 ID 需保留，下次存草稿才能覆盖同一条。
   */
  draftId?: string;
}

export type SyncMapRecord = Record<string, SyncEntry>;

export function findSyncEntry(
  map: SyncMapRecord | null | undefined,
  platformId: string,
): SyncEntry | undefined {
  if (!map) return undefined;
  return map[platformId];
}

export function hasSyncEntry(
  map: SyncMapRecord | null | undefined,
  platformId: string,
): boolean {
  return findSyncEntry(map, platformId) !== undefined;
}

export function isUsableSyncId(id: string | null | undefined): boolean {
  const trimmed = (id ?? "").trim();
  return trimmed !== "" && trimmed !== "0" && trimmed !== "未知" && trimmed !== "(未知)";
}

export function findUsableSyncEntry(
  map: SyncMapRecord | null | undefined,
  platformId: string,
): SyncEntry | undefined {
  const entry = findSyncEntry(map, platformId);
  if (!entry || !isUsableSyncId(entry.id)) return undefined;
  return entry;
}

export function isBodyChanged(entry: SyncEntry | undefined, bodyHash: string): boolean {
  if (!entry) return false;
  return entry.bodyHash !== bodyHash;
}

export type FailureClass =
  | "not_configured"
  | "auth_invalid"
  | "platform_error"
  | "network_error"
  | "not_implemented"
  | "unknown";

export type PublishOperation = "create" | "update";

export interface PublishResult {
  success: boolean;
  post_id: string;
  edit_url: string;
  message: string;
  failure_class?: FailureClass;
  operation: PublishOperation;
  isPublished: boolean;
  /** 草稿模式落库的远端草稿 ID（目前仅小红书返回）。 */
  draftId?: string;
}

export type SyncOutcome = "running" | "success" | "failure" | "cancelled";

export interface SyncLogEntry {
  timestamp: string;
  articleId: string;
  platformId: string;
  platformName: string;
  stage: string;
  outcome: SyncOutcome;
  failureClass?: FailureClass;
  message: string;
  draftUrl?: string;
  operation?: PublishOperation;
  isPublished?: boolean;
}