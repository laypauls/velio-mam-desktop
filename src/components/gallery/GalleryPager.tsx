import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from "lucide-react"
import { useId, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { formatNumber } from "@/lib/format"

/** 쪽 번호 버튼 목록 — 처음·끝과 현재 ±2 만 두고 사이는 줄임표. 총계가 추정치면 마지막 쪽은 넣지 않는다 */
export function pageNumbers(current: number, pageCount: number, estimate: boolean): (number | "gap")[] {
  const shown = new Set<number>([0])
  if (!estimate) shown.add(pageCount - 1)
  for (let p = current - 2; p <= current + 2; p++) if (p >= 0 && p < pageCount) shown.add(p)
  const sorted = [...shown].sort((a, b) => a - b)
  const out: (number | "gap")[] = []
  sorted.forEach((p, i) => {
    if (i > 0 && p - sorted[i - 1] > 1) out.push("gap")
    out.push(p)
  })
  if (estimate && sorted[sorted.length - 1] < pageCount - 1) out.push("gap")
  return out
}

/** 쪽 번호 입력 — 1..max 의 정수만 받는다. 틀리면 `aria-invalid` + 인라인 안내 */
function PageJump({ pageCount, onPage }: { pageCount: number; onPage: (page: number) => void }) {
  const { t } = useTranslation()
  const errorId = useId()
  const [value, setValue] = useState("")
  const [invalid, setInvalid] = useState(false)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    const n = /^\d+$/.test(value.trim()) ? Number(value) : NaN
    if (!(n >= 1 && n <= pageCount)) return setInvalid(true)
    setInvalid(false)
    setValue("")
    onPage(n - 1)
  }
  return (
    <form onSubmit={submit} className="flex items-center gap-1" noValidate>
      <Input
        value={value}
        onChange={(e) => {
          setValue(e.target.value)
          setInvalid(false)
        }}
        inputMode="numeric"
        aria-label={t("gallery.pager.input")}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errorId : undefined}
        className="h-8 w-20 text-center"
      />
      <Button type="submit" size="sm" variant="outline">
        {t("gallery.pager.go")}
      </Button>
      {invalid && (
        <p id={errorId} className="text-xs text-destructive break-keep">
          {t("gallery.pager.invalid", { max: formatNumber(pageCount) })}
        </p>
      )}
    </form>
  )
}

interface Props {
  /** 0-기준 쪽 번호 */
  page: number
  pageCount: number
  /** 이 쪽의 전역 범위 [start, end) */
  range: readonly [number, number]
  total: number
  /** 총계가 추정치(의미 검색) — 마지막 쪽 이동을 숨기고 "약 N쪽" 으로 표시한다 */
  totalIsEstimate: boolean
  onPage: (page: number) => void
}

/** 페이지 모드의 쪽 이동 바: 범위 표시 · 처음/이전/번호/다음/마지막 · 번호 입력 */
export function GalleryPager({ page, pageCount, range, total, totalIsEstimate, onPage }: Props) {
  const { t } = useTranslation()
  const last = pageCount - 1
  return (
    <nav aria-label={t("gallery.pager.label")} className="flex flex-wrap items-center gap-x-4 gap-y-2 border-t border-border px-4 py-2">
      <span className="text-sm text-muted-foreground tabular-nums break-keep">
        {t(totalIsEstimate ? "gallery.pager.rangeEstimate" : "gallery.pager.range", {
          from: formatNumber(range[0] + 1),
          to: formatNumber(range[1]),
          total: formatNumber(total),
        })}
      </span>
      <div className="flex items-center gap-1">
        <Button size="icon-sm" variant="ghost" aria-label={t("gallery.pager.first")} disabled={page === 0} onClick={() => onPage(0)}>
          <ChevronsLeft aria-hidden="true" />
        </Button>
        <Button size="icon-sm" variant="ghost" aria-label={t("gallery.pager.prev")} disabled={page === 0} onClick={() => onPage(page - 1)}>
          <ChevronLeft aria-hidden="true" />
        </Button>
        {pageNumbers(page, pageCount, totalIsEstimate).map((p, i) =>
          p === "gap" ? (
            <span key={`gap${i}`} aria-hidden="true" className="px-1 text-muted-foreground">
              …
            </span>
          ) : (
            <Button
              key={p}
              size="sm"
              variant={p === page ? "secondary" : "ghost"}
              aria-label={t("gallery.pager.page", { page: formatNumber(p + 1) })}
              aria-current={p === page ? "page" : undefined}
              onClick={() => onPage(p)}
            >
              {formatNumber(p + 1)}
            </Button>
          )
        )}
        {totalIsEstimate && <span className="px-1 text-sm text-muted-foreground break-keep">{t("gallery.pager.pageCountEstimate", { count: formatNumber(pageCount) })}</span>}
        <Button size="icon-sm" variant="ghost" aria-label={t("gallery.pager.next")} disabled={page >= last} onClick={() => onPage(page + 1)}>
          <ChevronRight aria-hidden="true" />
        </Button>
        {!totalIsEstimate && (
          <Button size="icon-sm" variant="ghost" aria-label={t("gallery.pager.last")} disabled={page >= last} onClick={() => onPage(last)}>
            <ChevronsRight aria-hidden="true" />
          </Button>
        )}
      </div>
      <PageJump key={page} pageCount={pageCount} onPage={onPage} />
    </nav>
  )
}
