import { ArrowDownWideNarrow, ArrowUpNarrowWide, Download, Ellipsis } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useUpdater } from "@/hooks/useUpdater"
import { useViewStore } from "@/stores"
import { useUpdateStore } from "@/stores/update"
import { CARD_MENU_CONTENT_CLASS, CardDisplayItems } from "./CardDisplayItems"
import { useSortModel } from "./useSortModel"

/** 하위 메뉴가 좁은 창(288px)·낮은 창에서 잘리지 않게 한다 */
const SUB_CLASS = "relative max-h-(--radix-dropdown-menu-content-available-height) max-w-[calc(100vw-1rem)] overflow-y-auto"

/**
 * 상단 바 "더보기"(⋯) — 컨테이너 < 426px 에서 정렬 기준·정렬 방향·보기 방식·카드 표시·업데이트를 한 메뉴로 모은다.
 * 선택지·상태·동작·비활성 규칙은 넓은 폭의 컨트롤과 같은 소스(`useSortModel`, `viewStore`, `CardDisplayItems`, `useUpdater`)를 쓴다.
 * 이 버튼은 넓은 폭에서는 CSS 로 숨고, 반대로 나머지 컨트롤이 이 폭에서 숨는다(TopBar).
 */
export function TopBarMoreMenu() {
  const { t } = useTranslation()
  const { open: openUpdate } = useUpdater()
  const version = useUpdateStore((s) => s.available?.version)
  const viewMode = useViewStore((s) => s.viewMode)
  const setViewMode = useViewStore((s) => s.setViewMode)
  const { fields, sort, semantic, orderPinned, desc, onField, toggleDirection } = useSortModel()
  const updateLabel = version ? t("update.indicator", { version }) : null
  const label = updateLabel ? `${t("shell.more")} — ${updateLabel}` : t("shell.more")
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label={label} title={label} className="relative hidden @max-[426px]:inline-flex">
          <Ellipsis aria-hidden="true" />
          {version && <span data-testid="more-update-dot" aria-hidden="true" className="absolute top-1.5 right-1.5 size-2 rounded-full bg-primary" />}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-52">
        <DropdownMenuSub>
          <DropdownMenuSubTrigger disabled={semantic} title={semantic ? t("sort.semanticOnly") : undefined}>
            {t("sort.label")}
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent className={SUB_CLASS}>
            <DropdownMenuRadioGroup value={sort.field} onValueChange={onField}>
              {fields.map((f) => (
                <DropdownMenuRadioItem key={f} value={f}>
                  {t(`sort.field.${f}`)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuItem disabled={semantic || orderPinned} onSelect={toggleDirection}>
          {desc ? <ArrowDownWideNarrow aria-hidden="true" /> : <ArrowUpNarrowWide aria-hidden="true" />}
          {t("sort.direction", { dir: t(desc ? "sort.desc" : "sort.asc") })}
        </DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>{t("view.mode")}</DropdownMenuSubTrigger>
          <DropdownMenuSubContent className={SUB_CLASS}>
            <DropdownMenuRadioGroup value={viewMode} onValueChange={(v) => setViewMode(v === "list" ? "list" : v === "filmstrip" ? "filmstrip" : "grid")}>
              <DropdownMenuRadioItem value="grid">{t("view.grid")}</DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="list">{t("view.list")}</DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="filmstrip">{t("view.filmstrip")}</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger disabled={viewMode !== "grid"}>{t("cardDisplay.title")}</DropdownMenuSubTrigger>
          <DropdownMenuSubContent className={`${SUB_CLASS} ${CARD_MENU_CONTENT_CLASS}`}>
            <CardDisplayItems />
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        {updateLabel && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={openUpdate}>
              <Download aria-hidden="true" />
              {updateLabel}
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
