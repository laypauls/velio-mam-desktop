import { useTranslation } from "react-i18next"
import type { PluginPermission } from "@/lib/ipc-plugin"

/** 권한 → 번역 키. Record 라 권한이 추가되면 typecheck 가 여기 빠진 것을 잡는다. */
const PERMISSION_KEY: Record<PluginPermission, string> = {
  "asset.read.metadata": "settings.plugins.permission.assetReadMetadata",
  "asset.read.derived": "settings.plugins.permission.assetReadDerived",
  "kv.read": "settings.plugins.permission.kvRead",
  "kv.write": "settings.plugins.permission.kvWrite",
  "tag.write": "settings.plugins.permission.tagWrite",
  "export.write": "settings.plugins.permission.exportWrite",
}

/** 권한을 사용자 말(번역)로 바꾸는 함수 */
export function usePermissionLabel(): (permission: PluginPermission) => string {
  const { t } = useTranslation()
  return (permission) => t(PERMISSION_KEY[permission])
}
