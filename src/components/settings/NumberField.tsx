import { useId, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { errorKey } from "@/lib/errors"

interface Props {
  label: string
  /** 입력 뒤에 붙는 단위 (시간·분·개) */
  unit: string
  value: number
  min: number
  max: number
  /** 유효한 새 값이 확정(blur / Enter)되면 호출. reject 하면 인라인 오류로 보여준다 */
  onCommit: (value: number) => Promise<unknown>
}

/** 정수 설정 입력. 범위 검증은 필드 단위 인라인 오류(aria-invalid + aria-describedby)로, 저장은 blur/Enter 에서 한다. */
export function NumberField({ label, unit, value, min, max, onCommit }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const errorId = `${id}-error`
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const shown = draft ?? String(value)

  const commit = async () => {
    if (draft === null) return
    const n = Number(draft)
    if (draft.trim() === "" || !Number.isInteger(n) || n < min || n > max) {
      setError(t("settings.errorRange", { min, max }))
      return
    }
    setDraft(null)
    if (n === value) {
      setError(null)
      return
    }
    try {
      await onCommit(n)
      setError(null)
    } catch (e) {
      setDraft(String(n))
      setError(t(errorKey(e)))
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") void commit()
  }

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className="!items-start">
        {label}
      </Label>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          className="w-24"
          value={shown}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          onChange={(e) => {
            setDraft(e.target.value)
            setError(null)
          }}
          onBlur={() => void commit()}
          onKeyDown={onKeyDown}
        />
        <span className="text-sm text-muted-foreground">{unit}</span>
      </div>
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}
