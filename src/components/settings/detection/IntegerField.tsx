import { useId } from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

interface Props {
  label: string
  /** 입력 뒤에 붙는 단위 (없으면 생략) */
  unit?: string
  value: string
  min: number
  max: number
  onChange: (value: string) => void
  error?: string
}

/** 라벨 + 정수 입력 + 인라인 오류 (aria-invalid + aria-describedby) — 간격·감지기 옵션이 함께 쓴다. */
export function IntegerField({ label, unit, value, min, max, onChange, error }: Props) {
  const id = useId()
  const errorId = `${id}-error`
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
          value={value}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          onChange={(e) => onChange(e.target.value)}
        />
        {unit && <span className="text-sm text-muted-foreground">{unit}</span>}
      </div>
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </div>
  )
}
