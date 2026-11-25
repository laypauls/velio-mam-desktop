import { useRef, type KeyboardEvent, type MouseEvent } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { RATING_MAX, RATING_STEPS, StarGlyph } from "./RatingStars"

interface Props {
  /** 0..5 (0 = 미평가) */
  value: number
  onChange: (value: number) => void
  disabled?: boolean
  /** false 면 탭 순서에서 빠진다(카드 오버레이처럼 격자의 roving 탐색을 따르는 곳) */
  tabbable?: boolean
  /** true 면 마우스로 눌러도 가장 가까운 listbox(격자)가 포커스를 유지한다 */
  keepGridFocus?: boolean
  /** 어두운 오버레이 위에 그릴 때 빈 별 색을 맞춘다 */
  onOverlay?: boolean
  className?: string
  starClassName?: string
}

/**
 * 별점 입력 — `radiogroup` + 별마다 `radio`. 현재 값의 별을 다시 누르면 0(해제), 화살표 키로 값을 바꾼다.
 * 클릭·더블클릭은 부모(카드 선택·열기)로 번지지 않는다.
 */
export function RatingInput({ value, onChange, disabled = false, tabbable = true, keepGridFocus = false, onOverlay = false, className, starClassName }: Props) {
  const { t } = useTranslation()
  const refs = useRef<(HTMLButtonElement | null)[]>([])

  const commit = (next: number) => {
    onChange(next)
    refs.current[Math.max(next, 1) - 1]?.focus()
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (disabled) return
    if (e.key === "ArrowRight" || e.key === "ArrowUp") commit(Math.min(RATING_MAX, value + 1))
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") commit(Math.max(0, value - 1))
    else return
    e.preventDefault()
    e.stopPropagation()
  }
  const press = (n: number) => (e: MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation()
    if (keepGridFocus) e.currentTarget.closest<HTMLElement>("[role='listbox']")?.focus({ preventScroll: true })
    onChange(n === value ? 0 : n)
  }

  return (
    <div
      role="radiogroup"
      aria-label={t("rating.label")}
      aria-disabled={disabled || undefined}
      onKeyDown={onKeyDown}
      className={cn("inline-flex items-center gap-0.5", className)}
    >
      {RATING_STEPS.map((n) => (
        <button
          key={n}
          ref={(el) => {
            refs.current[n - 1] = el
          }}
          type="button"
          role="radio"
          aria-checked={n === value}
          aria-label={t("rating.value", { count: n })}
          title={t("rating.value", { count: n })}
          disabled={disabled}
          tabIndex={tabbable && n === Math.max(value, 1) ? 0 : -1}
          onClick={press(n)}
          onDoubleClick={(e) => e.stopPropagation()}
          onMouseDown={keepGridFocus ? (e) => e.preventDefault() : undefined}
          className="rounded-sm p-px outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
        >
          <StarGlyph filled={n <= value} onOverlay={onOverlay} className={cn("size-4", starClassName)} />
        </button>
      ))}
    </div>
  )
}
