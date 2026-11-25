import { ArrowLeft, Search } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { useSearchStore } from "@/stores"
import { SearchBox } from "./SearchBox"

/**
 * 상단 바의 검색 — 상단 바 컨테이너(`@container`) 폭이 420px 이상이면 검색 박스 그대로,
 * 미만이면 돋보기 아이콘 버튼으로 접힌다. 누르면 같은 SearchBox 가 상단 바 전체를 덮는 입력으로 펼쳐진다
 * (인스턴스는 하나라 입력·검색 상태가 그대로다). ← · Esc · 바깥 클릭(포커스 이탈)로 닫는다 — 닫아도 검색어는 유지된다.
 * 폭 판정은 CSS 컨테이너 쿼리가 하므로 `open` 이 남아 있어도 넓은 상태에서는 아무 영향이 없다.
 */
export function TopBarSearch() {
  const { t } = useTranslation()
  const query = useSearchStore((s) => s.query)
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (open) root.current?.querySelector("input")?.focus()
  }, [open])
  // 접힌 아이콘 — 검색어가 있으면 점 배지 + 툴팁(검색어)으로 검색 중임을 알린다
  const label = query === "" ? t("search.open") : `${t("search.open")} — ${t("search.active", { query })}`
  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        aria-label={label}
        title={label}
        aria-expanded={open}
        onClick={() => setOpen(true)}
        className="relative hidden @max-[420px]:inline-flex"
      >
        <Search aria-hidden="true" />
        {query !== "" && <span data-testid="search-active-dot" aria-hidden="true" className="absolute top-1.5 right-1.5 size-2 rounded-full bg-primary" />}
      </Button>
      <div
        ref={root}
        data-testid="top-search"
        data-open={open ? "true" : undefined}
        className={cn(
          // 평소 max-w-xs, SearchBox 가 확장(data-expanded)이면 max-w-xl — 폭 전환은 여기서 해야 오른쪽 필터 버튼이 함께 움직인다
          "flex min-w-0 max-w-xs flex-1 items-center gap-2 transition-[max-width] duration-150 has-data-[expanded=true]:max-w-xl motion-reduce:transition-none",
          open
            ? "@max-[420px]:absolute @max-[420px]:inset-0 @max-[420px]:z-20 @max-[420px]:max-w-none! @max-[420px]:bg-background @max-[420px]:px-2"
            : "@max-[420px]:hidden"
        )}
        onBlur={(e) => {
          if (open && !e.currentTarget.contains(e.relatedTarget as Node | null)) setOpen(false)
        }}
        onKeyDown={(e) => {
          // 자동완성 팝업이 Esc 를 먼저 쓰면(defaultPrevented) 닫지 않는다. type=search 의 Esc 기본 동작(값 비우기)도 막는다
          if (!open || e.key !== "Escape" || e.defaultPrevented) return
          e.preventDefault()
          setOpen(false)
        }}
      >
        {open && (
          <Button variant="ghost" size="icon" aria-label={t("search.close")} title={t("search.close")} onClick={() => setOpen(false)} className="hidden @max-[420px]:inline-flex">
            <ArrowLeft aria-hidden="true" />
          </Button>
        )}
        <SearchBox />
      </div>
    </>
  )
}
