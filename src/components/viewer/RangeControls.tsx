import { Brackets } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { rangeKeyLabel, type RangeKeyActionType } from "./playRangeKeys"
import type { RangeUi } from "./useRangeController"

/**
 * 구간 그룹 — `구간 [시작][끝] (개수) [구간만]`. [시작][끝] 은 구간 시작/끝 키와 같은 동작이고 툴팁에 현재 프리셋의 키를 보인다.
 * 구간이 0개이면 개수 배지와 "구간만" 토글을 숨긴다. "구간만" 은 단독 재생일 때만(연속 재생 중에는 재생 바의 토글이 맡는다).
 * 좁은 컨테이너에서는 "구간" 라벨·"구간만" 글자를 접근성 전용으로 줄이고 아이콘+툴팁만 남긴다.
 */
export function RangeControls({ ui }: { ui: RangeUi }) {
  const { t } = useTranslation()
  const count = ui.ranges.length
  const mark = (action: RangeKeyActionType, label: string, run: () => void, pending: boolean) => {
    const key = rangeKeyLabel(ui.preset, action)
    return (
      <Button
        size="icon-sm"
        variant={pending ? "secondary" : "ghost"}
        className="font-mono text-xs"
        aria-label={label}
        aria-pressed={pending}
        title={key ? t("viewer.withKey", { label, key }) : label}
        onClick={run}
      >
        {key}
      </Button>
    )
  }
  return (
    <div className="flex items-center gap-1">
      <span className="text-xs text-muted-foreground break-keep @max-lg:sr-only">{t("ranges.group")}</span>
      {mark("markIn", t("ranges.markIn"), ui.markIn, ui.pendingInMs !== null)}
      {mark("markOut", t("ranges.markOut"), ui.markOut, ui.pendingOutMs !== null)}
      {count > 0 && (
        <>
          <Badge variant="secondary" className="rounded-sm break-keep">
            {t("ranges.count", { count })}
          </Badge>
          {ui.canToggleRangesOnly && (
            <Button size="sm" variant={ui.rangesOnly ? "secondary" : "ghost"} aria-pressed={ui.rangesOnly} title={t("ranges.only")} onClick={ui.toggleRangesOnly}>
              <Brackets aria-hidden="true" />
              <span className="@max-lg:sr-only">{t("ranges.only")}</span>
            </Button>
          )}
        </>
      )}
    </div>
  )
}
