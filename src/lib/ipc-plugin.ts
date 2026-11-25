import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import {
  PLUGIN_CHANGED_EVENT,
  type ExportResult,
  type PluginChanged,
  type PluginCommandResult,
  type PluginContributions,
  type PluginInfo,
  type PluginPermission,
  type PluginPreview,
  type PluginTaskCount,
  type RunAllResult,
} from "./ipc-contract-plugin"

export { PLUGIN_CHANGED_EVENT, PLUGIN_PERMISSIONS } from "./ipc-contract-plugin"
export type {
  ChangeReason,
  ExportResult,
  PluginApi,
  PluginAssetKind,
  PluginChanged,
  PluginCommandInfo,
  PluginCommandResult,
  PluginContributions,
  PluginErrorCode,
  PluginExportProviderEntry,
  PluginInfo,
  PluginMenuItem,
  PluginPermission,
  PluginPreview,
  PluginProviderInfo,
  PluginTaskCount,
  RunAllResult,
} from "./ipc-contract-plugin"

// 플러그인 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-plugin.ts`.

/** 설치된 플러그인 목록 (깨진·비호환 포함) */
export function pluginList(): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>("plugin_list")
}

/** 설치 전 폴더 검사 — 매니페스트·권한·설치/업그레이드 구분. 아무것도 바꾸지 않는다 */
export function pluginInspectDir(dir: string): Promise<PluginPreview> {
  return invoke<PluginPreview>("plugin_inspect_dir", { dir })
}

/** 설치·업그레이드 — 미리보기와 id·버전이 다르면 invalid_input (검사 후 폴더 변경 방지) */
export function pluginInstall(
  dir: string,
  expectedId: string,
  expectedVersion: string,
  grantedPermissions: PluginPermission[],
): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugin_install", { dir, expectedId, expectedVersion, grantedPermissions })
}

export function pluginSetEnabled(pluginId: string, enabled: boolean): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugin_set_enabled", { pluginId, enabled })
}

/** 부여 권한을 통째로 교체 (매니페스트 요청 권한의 부분집합이어야 한다) */
export function pluginGrantPermissions(pluginId: string, permissions: PluginPermission[]): Promise<PluginInfo> {
  return invoke<PluginInfo>("plugin_grant_permissions", { pluginId, permissions })
}

/** 제거 — keepData 면 플러그인 저장 데이터를 남긴다 */
export function pluginUninstall(pluginId: string, keepData: boolean): Promise<void> {
  return invoke<void>("plugin_uninstall", { pluginId, keepData })
}

/** 선택 자산(0~10,000)에 명령 실행 */
export function pluginRunCommand(pluginId: string, commandId: string, assetIds: number[]): Promise<PluginCommandResult> {
  return invoke<PluginCommandResult>("plugin_run_command", { pluginId, commandId, assetIds })
}

/** 메뉴·내보내기 기여 항목 (활성+정상 플러그인만) */
export function pluginContributions(): Promise<PluginContributions> {
  return invoke<PluginContributions>("plugin_contributions")
}

/** 제공자를 전체 자산에 대해 대기열에 등록 */
export function pluginRunAll(pluginId: string, providerId: string): Promise<RunAllResult> {
  return invoke<RunAllResult>("plugin_run_all", { pluginId, providerId })
}

/** 내보내기 제공자로 선택 자산을 파일로 저장 */
export function pluginExport(pluginId: string, providerId: string, assetIds: number[], destPath: string): Promise<ExportResult> {
  return invoke<ExportResult>("plugin_export", { pluginId, providerId, assetIds, destPath })
}

/** 플러그인별 대기·실패 작업 수 — `jobs://progress` 의 `counts.plugin` 이 바뀌면 다시 조회한다 */
export function pluginTaskCounts(): Promise<PluginTaskCount[]> {
  return invoke<PluginTaskCount[]>("plugin_task_counts")
}

/** 실패 작업을 다시 대기열에 올리고 그 수를 돌려준다 */
export function pluginRetryFailed(pluginId: string): Promise<number> {
  return invoke<number>("plugin_retry_failed", { pluginId })
}

/** `plugin://changed` 구독 — 설치·활성·권한·자동 비활성 등 변경. 반환 함수로 해제한다 */
export function onPluginChanged(handler: (change: PluginChanged) => void): Promise<UnlistenFn> {
  return listen<PluginChanged>(PLUGIN_CHANGED_EVENT, (e) => handler(e.payload))
}
