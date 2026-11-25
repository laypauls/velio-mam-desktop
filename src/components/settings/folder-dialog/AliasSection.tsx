import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { FOLDER_ALIAS_MAX } from "./useFolderSettingsSave"

interface Props {
  value: string
  onChange: (value: string) => void
  tooLong: boolean
  /** 저장 실패 메시지 (별칭 축) */
  error: string | null
}

/** 별칭 입력 — 비우면 별칭을 제거한다. 길이·저장 오류는 입력 아래에 인라인으로 보인다. */
export function AliasSection({ value, onChange, tooLong, error }: Props) {
  const { t } = useTranslation()
  const inputId = useId()
  const errorId = useId()
  const invalid = tooLong || error !== null
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={inputId} className="text-sm font-medium">
        {t("settings.folders.aliasLabel")}
      </label>
      <Input
        id={inputId}
        value={value}
        autoComplete="off"
        placeholder={t("settings.folders.aliasPlaceholder")}
        aria-invalid={invalid}
        aria-describedby={invalid ? errorId : undefined}
        onChange={(e) => onChange(e.target.value)}
      />
      <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.aliasHint", { max: FOLDER_ALIAS_MAX })}</p>
      {invalid && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {tooLong ? t("settings.folders.aliasTooLong", { max: FOLDER_ALIAS_MAX }) : error}
        </p>
      )}
    </div>
  )
}
