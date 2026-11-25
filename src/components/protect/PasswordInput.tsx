import { useId, type ComponentProps } from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

interface Props extends Omit<ComponentProps<typeof Input>, "type" | "id"> {
  label: string
  /** 인라인 오류 — 있으면 aria-invalid + aria-describedby 로 연결한다 */
  error?: string | null
}

/** 라벨 + 비밀번호 입력 + 인라인 오류 (ui.md 폼 규칙). */
export function PasswordInput({ label, error, className, ...props }: Props) {
  const id = useId()
  const errorId = `${id}-error`
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className="!items-start">
        {label}
      </Label>
      <Input
        id={id}
        type="password"
        autoComplete="off"
        aria-invalid={error ? true : undefined}
        aria-describedby={error ? errorId : undefined}
        className={className}
        {...props}
      />
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}
