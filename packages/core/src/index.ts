export type { PublishOptions, SyncResult, PlatformAdapter } from "./types";
export type { CredentialKind, PlatformDefinition } from "./platforms/registry";
export { PLATFORM_REGISTRY, PLATFORM_IDS, findPlatform, platformName, ADAPTER_IMPLEMENTED_IDS, auditRegistry } from "./platforms/registry";
export type {
  SyncEntry,
  SyncMapRecord,
  FailureClass,
  PublishOperation,
  PublishResult,
  SyncOutcome,
  SyncLogEntry,
} from "./platforms/sync";
export { findSyncEntry, findUsableSyncEntry, isUsableSyncId, hasSyncEntry, isBodyChanged } from "./platforms/sync";
export type { Article } from "./articles";
export { articleLabel } from "./articles";
export { APP_SLOGAN } from "./constants";