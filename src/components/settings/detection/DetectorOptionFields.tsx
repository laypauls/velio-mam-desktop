import { useTranslation } from "react-i18next"
import type { DetectorOptionField } from "@/lib/ipc-contract"
import { IntegerField } from "./IntegerField"
import { RemoteSourceField } from "./RemoteSourceField"
import { REMOTE_KIND_OF } from "./remote-kinds"

interface Props {
  detectorId: string
  fields: DetectorOptionField[]
  /** 입력 중 문자열 (key → 값) */
  values: Record<string, string>
  /** 이미 번역된 오류 문구 (key → 문구) */
  errors: Record<string, string>
  onChange: (key: string, value: string) => void
}

/**
 * 감지기가 선언한 옵션 칸 — 라벨은 `watch.detector.<id>.option.<key>` (없으면 키 그대로). 감지기 추가에 UI 변경이 필요 없다.
 * `integer` 는 숫자 칸, `ref` 는 원격 접속 선택. `bool` 은 감지기 전용 칸으로 그리지 않는다 (공통 옵션은 별도 영역).
 */
export function DetectorOptionFields({ detectorId, fields, values, errors, onChange }: Props) {
  const { t } = useTranslation()
  const shown = fields.filter((f) => f.kind !== "bool" && (f.kind !== "ref" || REMOTE_KIND_OF[detectorId] !== undefined))
  if (shown.length === 0) return null
  return (
    <fieldset className="flex flex-col gap-3 rounded-lg border border-border p-3">
      <legend className="px-1 text-xs font-medium text-muted-foreground">{t("settings.folders.detection.optionsTitle")}</legend>
      {shown.map((f) => {
        const label = t(`watch.detector.${detectorId}.option.${f.key}`, { defaultValue: f.key })
        return f.kind === "ref" ? (
          <RemoteSourceField
            key={f.key}
            label={label}
            kind={REMOTE_KIND_OF[detectorId]}
            value={values[f.key] ?? ""}
            error={errors[f.key]}
            onChange={(v) => onChange(f.key, v)}
          />
        ) : (
          <IntegerField
            key={f.key}
            label={label}
            value={values[f.key] ?? String(f.default)}
            min={f.min}
            max={f.max}
            error={errors[f.key]}
            onChange={(v) => onChange(f.key, v)}
          />
        )
      })}
    </fieldset>
  )
}
