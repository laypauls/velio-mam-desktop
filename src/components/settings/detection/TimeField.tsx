import { useId } from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

interface Props {
  label: string
  value: string
  onChange: (value: string) => void
  error?: string
  /** 이 칸 자신의 오류 문구 없이 외부 오류(예: 시작=끝)에 연결할 때 */
  externalErrorId?: string
}

/** 라벨 + `HH:MM` 입력 + 인라인 오류 (aria-invalid + aria-describedby). */
export function TimeField({ label, value, onChange, error, externalErrorId }: Props) {
  const id = useId()
  const errorId = `${id}-error`
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className="!items-start">
        {label}
      </Label>
      <Input
        id={id}
        type="time"
        step={60}
        className="w-32"
        value={value}
        aria-invalid={error || externalErrorId ? true : undefined}
        aria-describedby={error ? errorId : externalErrorId}
        onChange={(e) => onChange(e.target.value)}
      />
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </div>
  )
}
