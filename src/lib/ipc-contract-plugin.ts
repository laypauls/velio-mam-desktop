/**
 * 플러그인 UI ↔ Rust 코어 계약. 정본: `docs/requests/2026-11/20261122-170245-architect-plugin-runtime-jobs-spec.md` §C3.
 * Rust 쪽: `crates/mam-plugin-runtime/src/api/{dto,mod}.rs`, `src-tauri/src/plugin_commands.rs`·`plugin_job_commands.rs`.
 * 공유 픽스처: `src-tauri/src/plugin_ipc_fixture.json` (양쪽 테스트가 같은 JSON 을 검증한다).
 * - 필드는 camelCase, 시각은 UTC Unix ms.
 */

// ─── 권한·제공자 ─────────────────────────────────────────────────────────────

/** 플러그인이 요청·부여받을 수 있는 권한 — Rust 의 권한 문자열과 같다 */
export const PLUGIN_PERMISSIONS = [
  "asset.read.metadata",
  "asset.read.derived",
  "kv.read",
  "kv.write",
  "tag.write",
  "export.write",
] as const
export type PluginPermission = (typeof PLUGIN_PERMISSIONS)[number]

export type PluginAssetKind = "image" | "video" | "audio"

export interface PluginProviderInfo {
  id: string
  title: string
  kind: "metadata" | "analysis" | "export"
  assetKinds: PluginAssetKind[]
  fileExtension: string | null
}

export interface PluginCommandInfo {
  id: string
  title: string
}

// ─── 설치된 플러그인·설치 미리보기 ───────────────────────────────────────────

export interface PluginInfo {
  id: string
  version: string
  displayName: string
  description: string | null
  status: "ok" | "broken" | "incompatible"
  enabled: boolean
  /** 서명 검증 없음 — 항상 true */
  unsigned: true
  /** 매니페스트가 요청한 권한 */
  permissions: PluginPermission[]
  /** 사용자가 승인한 권한 (유효 권한 = permissions ∩ granted) */
  granted: PluginPermission[]
  commands: PluginCommandInfo[]
  providers: PluginProviderInfo[]
  installedAt: number
  workerAvailable: boolean
}

export interface PluginPreview {
  id: string
  version: string
  displayName: string
  description: string | null
  permissions: PluginPermission[]
  commands: PluginCommandInfo[]
  providers: PluginProviderInfo[]
  unsigned: true
  installed: { version: string; granted: PluginPermission[] } | null
  change: "install" | "upgrade" | "reinstall" | "downgrade"
  addedPermissions: PluginPermission[]
  /** 설치돼 있지 않지만 같은 ID 의 보존 데이터(KV·작업)가 남아 있다 */
  hasKeptData: boolean
}

// ─── 기여 항목 (메뉴·내보내기) ───────────────────────────────────────────────

/** 활성+정상 플러그인의 명령 메뉴 항목 — `when` 은 UI 에서만 평가한다 (보안 경계 아님) */
export interface PluginMenuItem {
  pluginId: string
  commandId: string
  title: string
  when: string | null
}

export interface PluginExportProviderEntry {
  pluginId: string
  providerId: string
  title: string
  fileExtension: string
  assetKinds: string[]
}

export interface PluginContributions {
  menuItems: PluginMenuItem[]
  exportProviders: PluginExportProviderEntry[]
}

// ─── 실행 결과 ───────────────────────────────────────────────────────────────

/** 제어 문자 제거·200자 이하로 정리된 메시지 (없으면 null) */
export interface PluginCommandResult {
  message: string | null
}

export interface RunAllResult {
  started: boolean
}

export interface ExportResult {
  bytesWritten: number
}

export interface PluginTaskCount {
  pluginId: string
  pending: number
  failed: number
}

// ─── 이벤트·오류 ─────────────────────────────────────────────────────────────

export const PLUGIN_CHANGED_EVENT = "plugin://changed"

export type ChangeReason =
  | "installed"
  | "upgraded"
  | "uninstalled"
  | "enabled"
  | "disabled"
  | "autoDisabled"
  | "permissionsChanged"

export interface PluginChanged {
  pluginId: string
  reason: ChangeReason
}

/** 플러그인 전용 오류 코드 — `src-tauri/src/plugin_errors.rs` 의 매핑과 같다 */
export type PluginErrorCode =
  | "plugin_invalid"
  | "plugin_incompatible"
  | "plugin_disabled"
  | "plugin_permission_denied"
  | "plugin_timeout"
  | "plugin_error"

// ─── API (래퍼 시그니처 — `ipc-plugin.ts`) ───────────────────────────────────

export interface PluginApi {
  pluginList(): Promise<PluginInfo[]>
  pluginInspectDir(dir: string): Promise<PluginPreview>
  pluginInstall(dir: string, expectedId: string, expectedVersion: string, grantedPermissions: PluginPermission[]): Promise<PluginInfo>
  pluginSetEnabled(pluginId: string, enabled: boolean): Promise<PluginInfo>
  pluginGrantPermissions(pluginId: string, permissions: PluginPermission[]): Promise<PluginInfo>
  pluginUninstall(pluginId: string, keepData: boolean): Promise<void>
  pluginRunCommand(pluginId: string, commandId: string, assetIds: number[]): Promise<PluginCommandResult>
  pluginContributions(): Promise<PluginContributions>
  pluginRunAll(pluginId: string, providerId: string): Promise<RunAllResult>
  pluginExport(pluginId: string, providerId: string, assetIds: number[], destPath: string): Promise<ExportResult>
  pluginTaskCounts(): Promise<PluginTaskCount[]>
  pluginRetryFailed(pluginId: string): Promise<number>
}
