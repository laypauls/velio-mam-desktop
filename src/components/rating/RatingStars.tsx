import { Star } from "lucide-react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"

export const RATING_MAX = 5
export const RATING_STEPS = [1, 2, 3, 4, 5] as const

/** 별 한 칸 — 채움 여부만 다르다. 색은 `--rating` 토큰(즐겨찾기 하트와 다른 색상). */
export function StarGlyph({ filled, className, onOverlay = false }: { filled: boolean; className?: string; onOverlay?: boolean }) {
  const empty = onOverlay ? "text-overlay-foreground/70" : "text-muted-foreground"
  return <Star aria-hidden="true" className={cn("shrink-0", filled ? "fill-rating text-rating" : empty, className)} />
}

/** 읽기 전용 별점 표시 — 접근성 이름은 "별점 N점" 한 개(개별 별은 숨김). 0(미평가)은 호출부가 숨길지 정한다. */
export function RatingStars({ value, className, starClassName }: { value: number; className?: string; starClassName?: string }) {
  const { t } = useTranslation()
  return (
    <span role="img" aria-label={t("rating.value", { count: value })} className={cn("inline-flex items-center gap-px", className)}>
      {RATING_STEPS.map((n) => (
        <StarGlyph key={n} filled={n <= value} className={cn("size-3", starClassName)} />
      ))}
    </span>
  )
}
