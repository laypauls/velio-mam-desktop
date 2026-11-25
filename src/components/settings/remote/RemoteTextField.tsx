import { useId, type ComponentProps } from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

interface Props extends Omit<ComponentProps<typeof Input>, "onChange" | "value" | "id"> {
  label: string
  value: string
  onValueChange: (value: string) => void
  /** 이미 번역된 인라인 오류 */
  error?: string
  /** 입력 아래 보조 설명 */
  hint?: string
  /** 입력 칸에만 붙는 클래스 (`className` 은 바깥 묶음) */
  inputClassName?: string
}

/** 라벨 + 입력 + 인라인 오류 (aria-invalid + aria-describedby). 비밀 입력도 이 컴포넌트를 쓴다 (`type="password"`). */
export function RemoteTextField({ label, value, onValueChange, error, hint, className, inputClassName, ...rest }: Props) {
  const id = useId()
  const errorId = `${id}-error`
  const hintId = `${id}-hint`
  const describedBy = [error ? errorId : null, hint ? hintId : null].filter(Boolean).join(" ")
  return (
    <div className={cn("flex min-w-0 flex-col gap-1.5", className)}>
      <Label htmlFor={id} className="!items-start">
        {label}
      </Label>
      <Input
        {...rest}
        id={id}
        className={inputClassName}
        value={value}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy || undefined}
        onChange={(e) => onValueChange(e.target.value)}
      />
      {hint && (
        <p id={hintId} className="text-xs text-muted-foreground break-keep">
          {hint}
        </p>
      )}
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </div>
  )
}
