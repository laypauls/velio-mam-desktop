import { useId, type ReactNode } from "react"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { cn } from "@/lib/utils"

export interface CardOption<T extends string> {
  value: T
  /** 라디오의 접근 가능한 이름이자 카드 제목 */
  label: string
  hint?: string
  /** 장식용 미리보기 — 스크린리더에서는 숨겨진다 */
  preview: ReactNode
}

interface Props<T extends string> {
  /** 라디오 그룹의 접근 가능한 이름이자 위쪽 제목 */
  title: string
  description?: string
  value: T
  options: readonly CardOption<T>[]
  onChange: (value: T) => void
  /** 충돌 경고 등을 라디오 그룹에 연결한다 */
  invalid?: boolean
  describedBy?: string
  /** 열 배치 (Tailwind grid-cols 클래스) */
  columnsClassName?: string
}

/**
 * 미리보기 카드로 고르는 단일 선택 — "화면 테마"(ThemeSection)와 같은 접근성 계약: `RadioGroup` + 카드 전체가 라벨, 선택은 `aria-checked`.
 * 선택 즉시 `onChange` 를 부르며 저장 방식은 호출자가 정한다.
 */
export function CardRadioGroup<T extends string>({ title, description, value, options, onChange, invalid, describedBy, columnsClassName }: Props<T>) {
  const base = useId()
  return (
    <div className="flex flex-col gap-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        {description && <span className="text-xs text-muted-foreground break-keep">{description}</span>}
      </div>
      <RadioGroup
        aria-label={title}
        aria-invalid={invalid || undefined}
        aria-describedby={describedBy}
        value={value}
        onValueChange={(v) => onChange(v as T)}
        className={cn("grid-cols-2 gap-2", columnsClassName)}
      >
        {options.map((o) => (
          <label
            key={o.value}
            htmlFor={`${base}-${o.value}`}
            className="flex cursor-pointer flex-col gap-2 rounded-lg border border-border bg-card p-2 has-[[aria-checked=true]]:border-primary has-[[aria-checked=true]]:ring-1 has-[[aria-checked=true]]:ring-primary has-[:focus-visible]:ring-3 has-[:focus-visible]:ring-ring/50"
          >
            <div aria-hidden="true" className="flex h-16 items-stretch overflow-hidden rounded-md border border-border bg-background">
              {o.preview}
            </div>
            <div className="flex items-start gap-2">
              <RadioGroupItem id={`${base}-${o.value}`} value={o.value} aria-label={o.label} className="mt-0.5" />
              <div className="flex min-w-0 flex-col">
                <span className="text-sm font-medium break-keep">{o.label}</span>
                {o.hint && <span className="text-xs text-muted-foreground break-keep">{o.hint}</span>}
              </div>
            </div>
          </label>
        ))}
      </RadioGroup>
    </div>
  )
}
