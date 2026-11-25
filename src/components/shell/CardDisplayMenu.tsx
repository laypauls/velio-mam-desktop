import { SlidersHorizontal } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { CARD_MENU_CONTENT_CLASS, CardDisplayItems } from "./CardDisplayItems"

/** 툴바 "표시 옵션" — 갤러리 카드의 정보 영역·오버레이 표시를 고른다. 격자 보기 전용(목록 보기에서는 비활성). 항목은 `CardDisplayItems`. */
export function CardDisplayMenu({ disabled }: { disabled: boolean }) {
  const { t } = useTranslation()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="icon" aria-label={t("view.cardDisplay")} title={t("view.cardDisplay")} disabled={disabled}>
          <SlidersHorizontal aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className={CARD_MENU_CONTENT_CLASS}>
        <CardDisplayItems />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
