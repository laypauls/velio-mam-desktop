import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Slider } from "@/components/ui/slider"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import { errorKey } from "@/lib/errors"
import { NumberField } from "../NumberField"
import { SettingRow } from "../SettingRow"
import { SimilarityBand } from "./SimilarityBand"

export const SIMILAR_MAX_RANGE = { min: 6, max: 200 } as const
const PERCENT_MAX = 99
const PERCENT_STEP = 5

/** 설정 > AI > 유사 미디어: 최대 표시 개수(숫자, blur/Enter 저장)와 최소 유사도(슬라이더, 놓을 때 저장). */
export function SimilarSection() {
  const { t } = useTranslation()
  const settings = useSettings().data
  const update = useUpdateSettings()
  // 드래그 중 표시값 — 저장은 놓을 때(onValueCommit)만 한다
  const [dragPercent, setDragPercent] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  if (!settings) return null
  const percent = dragPercent ?? Math.round(settings.similarMinSimilarity * 100)

  const commitPercent = async ([p]: number[]) => {
    try {
      await update.mutateAsync({ similarMinSimilarity: p / 100 })
      setError(null)
    } catch (e) {
      setError(t(errorKey(e)))
    } finally {
      setDragPercent(null)
    }
  }

  return (
    <section aria-label={t("ai.similarSettings.section")} className="flex flex-col gap-3">
      <h3 className="text-sm font-medium">{t("ai.similarSettings.section")}</h3>
      <div className="flex flex-col gap-1">
        <NumberField
          label={t("ai.similarSettings.max")}
          unit={t("settings.unit.items")}
          value={settings.similarMaxResults}
          min={SIMILAR_MAX_RANGE.min}
          max={SIMILAR_MAX_RANGE.max}
          onCommit={(similarMaxResults) => update.mutateAsync({ similarMaxResults })}
        />
        <p className="text-xs break-keep text-muted-foreground">{t("ai.similarSettings.maxHint")}</p>
      </div>
      <SettingRow label={t("ai.similarSettings.min")} description={t("ai.similarSettings.minHint")}>
        <div className="flex w-56 items-center gap-3">
          <Slider
            aria-label={t("ai.similarSettings.min")}
            min={0}
            max={PERCENT_MAX}
            step={PERCENT_STEP}
            value={[percent]}
            onValueChange={([p]) => setDragPercent(p)}
            onValueCommit={(v) => void commitPercent(v)}
          />
          <span className="w-10 shrink-0 text-right text-sm tabular-nums">{t("ai.similarSettings.minValue", { percent })}</span>
        </div>
      </SettingRow>
      <div className="max-w-xl">
        <SimilarityBand percent={percent} />
      </div>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </section>
  )
}
