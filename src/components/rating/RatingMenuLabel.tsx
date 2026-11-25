import { useTranslation } from "react-i18next"
import { RATING_STEPS, StarGlyph } from "./RatingStars"

/** 메뉴 항목 내용 — 별 N개(시각)와 "별점 N점"(스크린리더·텍스트 검색용). */
export function RatingMenuLabel({ value }: { value: number }) {
  const { t } = useTranslation()
  return (
    <>
      <span aria-hidden="true" className="inline-flex items-center gap-px">
        {RATING_STEPS.map((n) => (
          <StarGlyph key={n} filled={n <= value} className="size-3.5" />
        ))}
      </span>
      <span className="sr-only">{t("rating.value", { count: value })}</span>
    </>
  )
}

/** 메뉴에 나열하는 순서 — ★5 … ★1 */
export const RATING_MENU_VALUES = [5, 4, 3, 2, 1] as const
