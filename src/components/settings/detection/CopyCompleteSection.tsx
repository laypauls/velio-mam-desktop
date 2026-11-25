import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"

interface Props {
  /** 복사 완료 확인에 배타적 열기를 쓰는가 (기본 켜짐) */
  exclusive: boolean
  disabled?: boolean
  onExclusiveChange: (value: boolean) => void
}

/**
 * 복사 완료 확인 — 크기·수정 시각이 여러 번 같고(고정 규칙) 선택적으로 읽기 전용 배타적 열기까지 통과해야 반영한다.
 * 이 옵션은 감지기 방식(자동 포함)과 무관하게 적용된다.
 */
export function CopyCompleteSection({ exclusive, disabled, onExclusiveChange }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const hintId = `${id}-hint`
  return (
    <section className="flex flex-col gap-2" aria-labelledby={`${id}-title`}>
      <h4 id={`${id}-title`} className="text-sm font-semibold">
        {t("settings.folders.detection.copySection")}
      </h4>
      <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.detection.copyBase")}</p>
      <div className="flex items-start gap-3">
        <Switch id={id} checked={exclusive} disabled={disabled} aria-describedby={hintId} onCheckedChange={onExclusiveChange} />
        <div className="flex min-w-0 flex-col gap-1">
          <Label htmlFor={id} className="!items-start">
            {t("settings.folders.detection.exclusiveLabel")}
          </Label>
          <p id={hintId} className="text-xs text-muted-foreground break-keep">
            {t("settings.folders.detection.exclusiveHint")}
          </p>
        </div>
      </div>
    </section>
  )
}
