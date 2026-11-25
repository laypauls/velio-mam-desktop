import { Copy, Download, ExternalLink, FolderInput, FolderOpen, Trash2, type LucideIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useCopyToClipboard, useOriginalActions } from "@/hooks/useReuse"
import { useSettings } from "@/hooks/useSettings"
import type { Availability } from "@/lib/ipc-contract"
import { useActionsStore, useProtectStore } from "@/stores"

/** 오프라인·파일 없음이면 원본 열기/탐색기 보기를 막는 이유 (없으면 null). */
export function unavailableReason(availability: Availability, t: (key: string) => string): string | null {
  if (availability === "offline") return t("reuse.offlineReason")
  if (availability === "missing") return t("reuse.missingReason")
  return null
}

export interface ActionItem {
  id: string
  label: string
  icon: LucideIcon
  run: () => void
  disabled: boolean
  /** 비활성 이유(툴팁 2줄째) — 가용성 때문에 막힌 항목에만 있다 */
  reason: string | null
  destructive?: boolean
  /** 원본 이동·삭제 그룹 — 메뉴에서 구분선으로 나뉜다 */
  group: "base" | "original"
}

/** 우선순위 순서(넘치면 뒤에서부터 ⋯ 메뉴로): 열기, 탐색기, 파일 복사, 내보내기, (설정 시) 원본 이동·삭제. */
export function useAssetActionItems(assetId: number, availability: Availability): ActionItem[] {
  const { t } = useTranslation()
  const copy = useCopyToClipboard()
  const original = useOriginalActions()
  const openExport = useActionsStore((s) => s.openExport)
  const askOriginals = useProtectStore((s) => s.askOriginals)
  const originalActions = useSettings().data?.originalActionsEnabled ?? false
  const reason = unavailableReason(availability, t)
  const blocked = reason !== null

  const items: ActionItem[] = [
    { id: "open", label: t("detail.open"), icon: ExternalLink, run: () => original.open(assetId), disabled: blocked, reason, group: "base" },
    { id: "reveal", label: t("detail.reveal"), icon: FolderOpen, run: () => original.reveal(assetId), disabled: blocked, reason, group: "base" },
    { id: "copy", label: t("detail.copyFile"), icon: Copy, run: () => copy.mutate([assetId]), disabled: copy.isPending, reason: null, group: "base" },
    { id: "export", label: t("detail.exportAction"), icon: Download, run: () => openExport([assetId]), disabled: false, reason: null, group: "base" },
  ]
  if (originalActions) {
    items.push(
      { id: "move", label: t("detail.moveOriginal"), icon: FolderInput, run: () => askOriginals({ kind: "move", assetIds: [assetId] }), disabled: blocked, reason, group: "original" },
      { id: "delete", label: t("detail.deleteOriginal"), icon: Trash2, run: () => askOriginals({ kind: "delete", assetIds: [assetId] }), disabled: blocked, reason, group: "original", destructive: true }
    )
  }
  return items
}
