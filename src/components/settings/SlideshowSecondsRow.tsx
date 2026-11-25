import { useTranslation } from "react-i18next"
import { useUpdateSettings } from "@/hooks/useSettings"
import { NumberField } from "./NumberField"

export const SLIDESHOW_SECONDS_RANGE = { min: 1, max: 60 } as const

/** 설정 > 보기·동작: 컬렉션 연속 재생에서 이미지 한 장을 보여 주는 시간(초). blur/Enter 로 저장하고 범위 밖이면 인라인 오류. */
export function SlideshowSecondsRow({ value }: { value: number }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  return (
    <div className="flex flex-col gap-1">
      <NumberField
        label={t("settings.view.slideshowSeconds")}
        unit={t("settings.unit.seconds")}
        value={value}
        min={SLIDESHOW_SECONDS_RANGE.min}
        max={SLIDESHOW_SECONDS_RANGE.max}
        onCommit={(slideshowImageSeconds) => update.mutateAsync({ slideshowImageSeconds })}
      />
      <p className="text-xs break-keep text-muted-foreground">{t("settings.view.slideshowSecondsHint")}</p>
    </div>
  )
}
