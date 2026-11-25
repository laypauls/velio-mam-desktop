import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { useUpdateSettings } from "@/hooks/useSettings"

/**
 * 설정 > 보기·동작: 동영상·음성을 열 때 재생 방식. 기본은 정지 상태로 열기이며, 컬렉션 연속 재생은 이 설정과 무관하게 항상 자동 재생한다.
 * 기본 동작이 바뀌는 항목이라 스위치 대신 두 선택지를 모두 보여 준다.
 */
export function OpenAutoplayRow({ value }: { value: boolean }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const noteId = useId()
  const pausedId = useId()
  const autoId = useId()
  return (
    <div role="group" aria-labelledby={`${noteId}-title`} className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <span id={`${noteId}-title`} className="text-sm font-medium">
          {t("settings.view.openAutoplay")}
        </span>
        <span id={noteId} className="text-xs text-muted-foreground break-keep">
          {t("settings.view.openAutoplayHint")}
        </span>
      </div>
      <RadioGroup
        aria-label={t("settings.view.openAutoplay")}
        aria-describedby={noteId}
        value={value ? "autoplay" : "paused"}
        onValueChange={(v) => update.mutate({ mediaOpenAutoplay: v === "autoplay" })}
      >
        <div className="flex items-center gap-2">
          <RadioGroupItem id={pausedId} value="paused" />
          <Label htmlFor={pausedId}>{t("settings.view.openAutoplayPaused")}</Label>
        </div>
        <div className="flex items-center gap-2">
          <RadioGroupItem id={autoId} value="autoplay" />
          <Label htmlFor={autoId}>{t("settings.view.openAutoplayOn")}</Label>
        </div>
      </RadioGroup>
    </div>
  )
}
