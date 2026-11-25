import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { formatNumber } from "@/lib/format"
import type { MediaType, MediaTypeCounts, MediaTypeSet } from "@/lib/ipc-contract"

/** 표시 순서 (동영상 · 이미지 · 음성). */
export const MEDIA_TYPE_ORDER: readonly MediaType[] = ["video", "image", "audio"]

export const ALL_MEDIA_TYPES: MediaTypeSet = { image: true, video: true, audio: true }

export function noMediaTypeSelected(v: MediaTypeSet): boolean {
  return !v.image && !v.video && !v.audio
}

export function isAllMediaTypes(v: MediaTypeSet): boolean {
  return v.image && v.video && v.audio
}

interface Props {
  value: MediaTypeSet
  onChange: (next: MediaTypeSet) => void
  /** 있으면 유형 이름 뒤에 현재 수집 개수를 보인다. */
  counts?: MediaTypeCounts
  disabled?: boolean
  /** 인라인 에러 메시지 (aria-invalid + aria-describedby 로 체크박스와 연결). */
  error?: string | null
}

/** 스캔 대상 유형 체크박스 3개 (+ 인라인 에러). 폴더 추가·대상 변경 Dialog 공용. */
export function MediaTypeFields({ value, onChange, counts, disabled, error }: Props) {
  const { t } = useTranslation()
  const groupId = useId()
  const errorId = useId()
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-1 text-sm font-medium">{t("folders.types.label")}</legend>
      <div className="flex flex-wrap gap-x-5 gap-y-2">
        {MEDIA_TYPE_ORDER.map((type) => {
          const id = `${groupId}-${type}`
          return (
            <div key={type} className="flex items-center gap-2">
              <Checkbox
                id={id}
                checked={value[type]}
                disabled={disabled}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? errorId : undefined}
                onCheckedChange={(v) => onChange({ ...value, [type]: v === true })}
              />
              <Label htmlFor={id}>
                {t(`asset.type.${type}`)}
                {counts && <span className="text-muted-foreground"> ({formatNumber(counts[type])})</span>}
              </Label>
            </div>
          )
        })}
      </div>
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </fieldset>
  )
}
