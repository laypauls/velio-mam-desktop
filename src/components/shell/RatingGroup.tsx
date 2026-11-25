import { Star } from "lucide-react"
import { useTranslation } from "react-i18next"
import { RATING_STEPS, StarGlyph } from "@/components/rating/RatingStars"
import { useRatingCounts } from "@/hooks/useRatingCounts"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import { useSearchStore } from "@/stores"

/** 표시 순서 — ★5 … ★1, 미평가 */
const ORDER = [5, 4, 3, 2, 1, 0] as const

/** 사이드바 "별점" 그룹 항목들 — 정확히 N점 보기 + 개수. 표시 여부(설정)는 호출부가 정하고, 안 그릴 때는 이 컴포넌트를 마운트하지 않아 개수 질의도 하지 않는다. */
export function RatingList() {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const counts = useRatingCounts(true).data

  return (
    <ul>
      {ORDER.map((n) => {
        const active = view.kind === "rating" && view.value === n
        const label = n === 0 ? t("sidebar.ratingUnrated") : t("rating.value", { count: n })
        return (
          <li key={n}>
            <button
              type="button"
              aria-current={active ? "true" : undefined}
              onClick={() => setView({ kind: "rating", value: n })}
              className={cn("flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted", active && "bg-accent text-accent-foreground")}
            >
              {n === 0 ? (
                <>
                  <Star aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate">{label}</span>
                </>
              ) : (
                <>
                  <span aria-hidden="true" className="inline-flex flex-1 items-center gap-px">
                    {RATING_STEPS.map((s) => (
                      <StarGlyph key={s} filled={s <= n} className="size-3.5" />
                    ))}
                  </span>
                  <span className="sr-only">{label}</span>
                </>
              )}
              {counts !== undefined && <span className="text-xs text-muted-foreground tabular-nums">{formatNumber(counts[n] ?? 0)}</span>}
            </button>
          </li>
        )
      })}
    </ul>
  )
}
