import { Download, Puzzle } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { ContextMenuItem, ContextMenuSeparator, ContextMenuShortcut } from "@/components/ui/context-menu"
import { usePluginContributions } from "@/hooks/usePluginContributions"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatBytes } from "@/lib/format"
import type { AssetSummary } from "@/lib/ipc-contract"
import { pluginExport, pluginRunCommand, type PluginExportProviderEntry, type PluginMenuItem } from "@/lib/ipc-plugin"
import { evalWhen, type WhenContext } from "@/lib/when"

/** 플러그인 내보내기 한 번에 담을 수 있는 자산 수 상한 — Rust `MAX_EXPORT_ASSETS`(crates/mam-plugin-runtime/src/jobs/export.rs)와 같아야 한다 */
export const MAX_PLUGIN_EXPORT_ASSETS = 5_000

/** `when` 식에 공급하는 컨텍스트 — `asset.*` 는 우클릭한 자산 기준, `selection.count` 는 선택 전체 */
export function menuWhenContext(asset: AssetSummary, ids: readonly number[]): WhenContext {
  return {
    "selection.count": ids.length,
    "asset.kind": asset.mediaType,
    "asset.offline": asset.availability !== "online",
  }
}

/**
 * 컨텍스트 메뉴의 "플러그인" 그룹 — `when` 이 참인 기여 명령, 이어서 내보내기 제공자. 하나도 없으면 항목·구분선 모두 렌더하지 않는다.
 * 앞뒤 구분선은 이 그룹이 소유한다(내보내기 항목 바로 앞에 놓인다).
 */
export function PluginMenuSlot({ asset, ids }: { asset: AssetSummary; ids: number[] }) {
  const { t } = useTranslation()
  const api = useApi()
  const contributions = usePluginContributions().data
  const ctx = menuWhenContext(asset, ids)
  const items = (contributions?.menuItems ?? []).filter((m) => evalWhen(m.when, ctx))
  const providers = contributions?.exportProviders ?? []
  if (items.length === 0 && providers.length === 0) return null
  const tooMany = ids.length > MAX_PLUGIN_EXPORT_ASSETS

  const run = async (item: PluginMenuItem) => {
    try {
      const result = await pluginRunCommand(item.pluginId, item.commandId, ids)
      toast.success(result.message ?? t("pluginCommands.done", { title: item.title }))
    } catch (e) {
      toast.error(t(errorKey(e)))
    }
  }

  const exportWith = async (p: PluginExportProviderEntry) => {
    try {
      const ext = p.fileExtension.replace(/^\./, "")
      const dest = await api.pickSaveFile(`export.${ext}`, [{ name: p.title, extensions: [ext] }])
      if (dest === null) return // 취소 — 조용히 끝낸다
      const result = await pluginExport(p.pluginId, p.providerId, ids, dest)
      toast.success(t("pluginCommands.exportDone", { title: p.title, size: formatBytes(result.bytesWritten) }))
    } catch (e) {
      toast.error(t(errorKey(e)))
    }
  }

  return (
    <>
      <ContextMenuSeparator />
      {items.map((item) => (
        <ContextMenuItem key={`${item.pluginId}/${item.commandId}`} onSelect={() => void run(item)}>
          <Puzzle aria-hidden="true" />
          {item.title}
        </ContextMenuItem>
      ))}
      {providers.map((p) => (
        <ContextMenuItem key={`${p.pluginId}/export/${p.providerId}`} disabled={tooMany} onSelect={() => void exportWith(p)}>
          <Download aria-hidden="true" />
          {t("pluginCommands.exportItem", { title: p.title })}
          {tooMany && <ContextMenuShortcut>{t("pluginCommands.exportTooMany", { max: MAX_PLUGIN_EXPORT_ASSETS.toLocaleString("en-US") })}</ContextMenuShortcut>}
        </ContextMenuItem>
      ))}
      <ContextMenuSeparator />
    </>
  )
}
