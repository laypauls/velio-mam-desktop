import { ChevronDown, SearchX } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { Slider } from "@/components/ui/slider"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useUpdateSettings } from "@/hooks/useSettings"
import { errorKey } from "@/lib/errors"
import { SIMILAR_MAX_RANGE } from "@/components/settings/ai/SimilarSection"

const PERCENT_MAX = 99
/** 최대 개수 선택지 — 설정 화면 허용 범위(SIMILAR_MAX_RANGE) 안의 값만. 현재 값이 목록에 없으면 함께 보여 준다. */
const MAX_CHOICES = [12, 24, 48, 100]

interface Props {
  max: number
  minSimilarity: number
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** 기준 칩 "{p}% 이상 · 최대 {n}개 ▾" + Popover. 전역 설정(similarMinSimilarity·similarMaxResults)에 저장한다 — 슬라이더는 놓을 때 저장. */
export function SimilarCriteria({ max, minSimilarity, open, onOpenChange }: Props) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const [dragPercent, setDragPercent] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const percent = dragPercent ?? Math.round(minSimilarity * 100)
  const choices = [...new Set([...MAX_CHOICES, max])]
    .filter((n) => n >= SIMILAR_MAX_RANGE.min && n <= SIMILAR_MAX_RANGE.max)
    .sort((a, b) => a - b)

  const save = async (patch: { similarMinSimilarity: number } | { similarMaxResults: number }) => {
    try {
      await update.mutateAsync(patch)
      setError(null)
    } catch (e) {
      setError(t(errorKey(e)))
    } finally {
      setDragPercent(null)
    }
  }
  const chip =
    minSimilarity > 0
      ? t("ai.similar.criteria", {
          percent: Math.round(minSimilarity * 100),
          max,
        })
      : t("ai.similar.criteriaNoMin", { max })

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>
        <Button size="xs" variant="outline" className="max-w-full">
          <span className="truncate">{chip}</span>
          <ChevronDown aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="flex w-72 max-w-[calc(100vw-2rem)] flex-col gap-3">
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between text-sm">
            <span>{t("ai.similarSettings.min")}</span>
            <span className="tabular-nums">{t("ai.similarSettings.minValue", { percent })}</span>
          </div>
          <Slider
            aria-label={t("ai.similarSettings.min")}
            min={0}
            max={PERCENT_MAX}
            step={1}
            value={[percent]}
            onValueChange={([p]) => setDragPercent(p)}
            onValueCommit={([p]) => void save({ similarMinSimilarity: p / 100 })}
          />
        </div>
        <div className="flex flex-col gap-2">
          <span className="text-sm">{t("ai.similar.maxLabel")}</span>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={String(max)}
            aria-label={t("ai.similar.maxLabel")}
            // 선택된 항목을 다시 누르면 빈 값이 온다 — 무시
            onValueChange={(v) => v && void save({ similarMaxResults: Number(v) })}
          >
            {choices.map((n) => (
              <ToggleGroupItem key={n} value={String(n)}>
                {n}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
        <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.criteriaScope")}</p>
      </PopoverContent>
    </Popover>
  )
}

/** 하한(기준)이 있어 결과가 0건일 때의 빈 상태 — [기준 변경] 이 기준 Popover 를 연다. */
export function SimilarThresholdEmpty({
  minSimilarity,
  onChangeCriteria,
}: {
  minSimilarity: number
  onChangeCriteria: () => void
}) {
  const { t } = useTranslation()
  return (
    <div role="status" className="flex flex-col items-center gap-1.5 p-4 text-center">
      <SearchX aria-hidden="true" className="size-6 text-muted-foreground" />
      <p className="text-sm font-medium break-keep">
        {t("ai.similar.thresholdEmptyTitle", {
          percent: Math.round(minSimilarity * 100),
        })}
      </p>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.thresholdEmptyDescription")}</p>
      <Button size="sm" variant="outline" onClick={onChangeCriteria}>
        {t("ai.similar.changeCriteria")}
      </Button>
    </div>
  )
}
