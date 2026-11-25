import { useTranslation } from "react-i18next"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"

/** Radix Select 는 빈 문자열 값을 쓸 수 없다 — "제한 없음" 은 sentinel 로 렌더하고 저장 시 null 로 되돌린다 */
export const ANY = "__any__"
const LEVELS = [0, 1, 2, 3, 4, 5] as const

export function toSelectValue(v: number | null): string {
  return v === null ? ANY : String(v)
}

export function fromSelectValue(v: string): number | null {
  return v === ANY ? null : Number(v)
}

/** 최소 > 최대 이면 서로 바꾼다 (둘 다 값이 있을 때만). */
export function normalizeRatingRange(min: number | null, max: number | null): [number | null, number | null] {
  return min !== null && max !== null && min > max ? [max, min] : [min, max]
}

function RatingSelect({ value, label, onChange }: { value: number | null; label: string; onChange: (v: number | null) => void }) {
  const { t } = useTranslation()
  return (
    <Select value={toSelectValue(value)} onValueChange={(v) => onChange(fromSelectValue(v))}>
      <SelectTrigger aria-label={label} className="w-28">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ANY}>{t("filter.ratingAny")}</SelectItem>
        {LEVELS.map((n) => (
          <SelectItem key={n} value={String(n)}>
            {n === 0 ? t("rating.none") : t("filter.ratingStar", { n })}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

interface Props {
  label: string
  min: number | null
  max: number | null
  onApply: (min: number | null, max: number | null) => void
}

/** 필터 팝오버의 별점 범위 행 — 최소 ~ 최대(포함). 같은 값 = 정확히 N점, 0 = 미평가. */
export function FilterRatingRow({ label, min, max, onApply }: Props) {
  const { t } = useTranslation()
  const apply = (a: number | null, b: number | null) => onApply(...normalizeRatingRange(a, b))
  return (
    <div role="group" aria-label={label} className="flex flex-col !items-start gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <div className="flex items-center gap-2">
        <RatingSelect value={min} label={t("filter.ratingMin")} onChange={(v) => apply(v, max)} />
        <span aria-hidden="true">{t("filter.ratingTo")}</span>
        <RatingSelect value={max} label={t("filter.ratingMax")} onChange={(v) => apply(min, v)} />
      </div>
    </div>
  )
}
