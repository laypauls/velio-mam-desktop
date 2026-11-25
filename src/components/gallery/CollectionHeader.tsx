import { Images, Play, Repeat } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useCollections } from "@/hooks/useLibrary"
import { useStartPlayback } from "@/hooks/usePlayQueue"
import { formatNumber } from "@/lib/format"
import { usePlaybackStore, useSearchStore } from "@/stores"

/** 컬렉션 보기 위쪽 머리글 — 이름·개수, ▶ 재생(처음부터), 반복 토글. 컬렉션 보기가 아니면 그리지 않는다. */
export function CollectionHeader() {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const collections = useCollections()
  const repeat = usePlaybackStore((s) => s.repeat)
  const toggleRepeat = usePlaybackStore((s) => s.toggleRepeat)
  const start = useStartPlayback()
  if (view.kind !== "collection") return null
  const current = collections.data?.find((c) => c.collectionId === view.collectionId)
  const repeatLabel = t(repeat === "all" ? "playback.repeatAll" : "playback.repeatOff")
  return (
    <header className="flex items-center gap-2 border-b border-border px-4 py-2">
      <Images aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
      <h2 className="min-w-0 truncate text-sm font-medium">{current?.name ?? ""}</h2>
      {current && <span className="text-xs text-muted-foreground tabular-nums">{t("collections.header.count", { count: formatNumber(current.assetCount) })}</span>}
      <Button size="sm" disabled={!current || current.assetCount === 0} title={t("collections.header.playTitle")} onClick={() => void start(view.collectionId)}>
        <Play aria-hidden="true" />
        {t("collections.header.play")}
      </Button>
      <Button size="sm" variant={repeat === "all" ? "secondary" : "outline"} aria-pressed={repeat === "all"} aria-label={t("playback.repeat")} title={repeatLabel} onClick={toggleRepeat}>
        <Repeat aria-hidden="true" />
        {repeatLabel}
      </Button>
    </header>
  )
}
