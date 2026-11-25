import { ArrowDownWideNarrow, ArrowUpDown, ArrowUpNarrowWide, GalleryHorizontalEnd, LayoutGrid, List } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useViewStore } from "@/stores"
import { CardDisplayMenu } from "./CardDisplayMenu"
import { useSortModel } from "./useSortModel"

export function SortControl() {
  const { t } = useTranslation()
  const { fields, sort, semantic, orderPinned, desc, onField, toggleDirection } = useSortModel(true)
  return (
    <div className="flex items-center gap-1" title={semantic ? t("sort.semanticOnly") : undefined}>
      {semantic && (
        <span id="sort-semantic-note" className="sr-only">
          {t("sort.semanticOnly")}
        </span>
      )}
      <Select
        value={sort.field}
        disabled={semantic}
        onValueChange={onField}
      >
        <SelectTrigger
          aria-label={t("sort.label")}
          aria-describedby={semantic ? "sort-semantic-note" : undefined}
          className="w-36 @max-[640px]:hidden"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {fields.map((f) => (
            <SelectItem key={f} value={f}>
              {t(`sort.field.${f}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {/* 상단 바 컨테이너 < 640px: Select 대신 아이콘 메뉴 (같은 선택지·같은 비활성 규칙). 숨김은 CSS(container query)라 두 형태가 DOM 에 함께 있다 */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="outline"
            size="icon"
            aria-label={t("sort.label")}
            aria-describedby={semantic ? "sort-semantic-note" : undefined}
            title={`${t("sort.label")}: ${t(`sort.field.${sort.field}`)}`}
            disabled={semantic}
            className="hidden @max-[640px]:inline-flex"
          >
            <ArrowUpDown aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuRadioGroup value={sort.field} onValueChange={onField}>
            {fields.map((f) => (
              <DropdownMenuRadioItem key={f} value={f}>
                {t(`sort.field.${f}`)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        variant="outline"
        size="icon"
        aria-label={t("sort.direction", { dir: t(desc ? "sort.desc" : "sort.asc") })}
        disabled={semantic || orderPinned}
        onClick={toggleDirection}
      >
        {desc ? <ArrowDownWideNarrow aria-hidden="true" /> : <ArrowUpNarrowWide aria-hidden="true" />}
      </Button>
    </div>
  )
}

const MODES = [
  { mode: "grid", icon: LayoutGrid, label: "view.grid" },
  { mode: "list", icon: List, label: "view.list" },
  { mode: "filmstrip", icon: GalleryHorizontalEnd, label: "view.filmstrip" },
] as const

/** 보기 방식 3단 전환(격자/목록/필름스트립) + 카드 표시 메뉴(격자에서만 활성). 좁은 폭에서는 더보기 메뉴의 라디오가 대신한다. */
export function ViewControls() {
  const { t } = useTranslation()
  const viewMode = useViewStore((s) => s.viewMode)
  const setViewMode = useViewStore((s) => s.setViewMode)
  return (
    <>
      {/* 단독 렌더(테스트·스토리)에서도 툴팁이 동작하도록 자체 Provider */}
      <TooltipProvider>
        <ToggleGroup
          type="single"
          variant="outline"
          spacing={0}
          value={viewMode}
          aria-label={t("view.mode")}
          // 눌린 항목을 다시 눌러도 해제되지 않는다 (빈 값은 무시)
          onValueChange={(v) => {
            const next = MODES.find((m) => m.mode === v)
            if (next) setViewMode(next.mode)
          }}
        >
          {MODES.map(({ mode, icon: Icon, label }) => (
            <Tooltip key={mode}>
              <TooltipTrigger asChild>
                <ToggleGroupItem value={mode} aria-label={t(label)}>
                  <Icon aria-hidden="true" />
                </ToggleGroupItem>
              </TooltipTrigger>
              <TooltipContent>{t(label)}</TooltipContent>
            </Tooltip>
          ))}
        </ToggleGroup>
      </TooltipProvider>
      <CardDisplayMenu disabled={viewMode !== "grid"} />
    </>
  )
}
