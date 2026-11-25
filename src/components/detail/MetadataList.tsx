import { ChevronRight } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useSettings } from "@/hooks/useSettings"
import { DEFAULT_CAPTURED_AT_FALLBACK, effectiveCapturedAt } from "@/lib/capturedAt"
import { formatBytes, formatDateTime, formatDuration, formatNumber } from "@/lib/format"
import type { AssetDetail } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"

type Row = [string, string | null]

function Rows({ rows }: { rows: Row[] }) {
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-sm">
      {rows
        .filter((r): r is [string, string] => r[1] !== null)
        .map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="break-keep text-muted-foreground">{k}</dt>
            <dd className="break-words select-text">{v}</dd>
          </div>
        ))}
    </dl>
  )
}

/**
 * 메타데이터 — 요약 한 줄(해상도 · 길이 · 크기) + 촬영일은 항상 보이고, 나머지는 "세부 정보" 로 접는다.
 * 값이 없는 행은 숨긴다. 숫자·날짜는 현재 표시 언어의 Intl (D13).
 */
export function MetadataList({ detail: d }: { detail: AssetDetail }) {
  const { t } = useTranslation()
  const open = useViewStore((s) => s.detailMetaOpen)
  const setOpen = useViewStore((s) => s.setDetailMetaOpen)
  const mode = useSettings().data?.capturedAtFallback ?? DEFAULT_CAPTURED_AT_FALLBACK
  const captured = effectiveCapturedAt(d, mode)
  const summary = [
    d.width && d.height ? `${formatNumber(d.width)} × ${formatNumber(d.height)}` : null,
    d.durationMs !== null ? formatDuration(d.durationMs) : null,
    formatBytes(d.fileSize),
  ].filter((v): v is string => v !== null)
  const capturedRow: Row = [t("detail.captured"), captured.at !== null ? `${formatDateTime(captured.at)}${captured.fallback ? ` ${t("detail.capturedFallbackMark")}` : ""}` : null]
  const detailRows: Row[] = [
    [t("detail.codec"), d.codec],
    [t("detail.audioCodec"), d.audioCodec],
    [t("detail.fps"), d.fps !== null ? formatNumber(d.fps) : null],
    [t("detail.bitrate"), d.bitrate !== null ? `${formatNumber(Math.round(d.bitrate / 1000))} kbps` : null],
    [t("detail.camera"), d.camera],
    [t("detail.volume"), d.volumeLabel],
    [t("detail.modified"), formatDateTime(d.modifiedAt)],
    ...Object.entries(d.extra),
  ]
  const count = detailRows.filter((r) => r[1] !== null).length

  return (
    <div className="flex flex-col gap-2">
      <p className="text-sm select-text">{summary.join(" · ")}</p>
      <Rows rows={[capturedRow]} />
      {count > 0 && (
        <Collapsible open={open} onOpenChange={setOpen} className="flex flex-col gap-2">
          <CollapsibleTrigger className="group flex items-center gap-1 self-start rounded-sm text-xs text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none">
            <ChevronRight aria-hidden="true" className="size-3.5 transition-transform group-data-[state=open]:rotate-90" />
            {t("detail.details", { count })}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Rows rows={detailRows} />
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  )
}
