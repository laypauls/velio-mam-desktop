import { Settings } from "lucide-react"
import { useTranslation } from "react-i18next"
import symbol from "@/assets/velio-symbol.svg"
import { useSettings } from "@/hooks/useSettings"
import { useProtectStore, useSearchStore } from "@/stores"
import { selectPanelVisible, useViewStore, type SidebarTab } from "@/stores/view"
import { RailButton } from "./RailButton"
import { effectiveSection, RAIL_SECTIONS, SIDEBAR_PANEL_ID, viewSection } from "./railSections"

/**
 * 고정 왼쪽 레일 — 창 맨 위부터 맨 아래까지. 로고(패널 숨김/표시) · 섹션 · 설정(유일한 설정 진입점).
 * 레일은 접히지 않는다. 설정 화면이 열려도 유지되며, 섹션을 누르면 설정을 닫고 그 섹션으로 간다.
 * z-index 를 주지 않는다 — 플레이어 전체 화면(z-40)이 레일을 덮어야 한다.
 */
export function NavRail() {
  const { t } = useTranslation()
  const ratingMenu = useSettings().data?.sidebarRatingMenu
  const tab = useViewStore((s) => s.sidebarTab)
  const panelVisible = useViewStore(selectPanelVisible)
  const toggleSidebar = useViewStore((s) => s.toggleSidebar)
  const selectSection = useViewStore((s) => s.selectSection)
  const showSection = useViewStore((s) => s.showSection)
  const view = useSearchStore((s) => s.view)
  const tagsActive = useSearchStore((s) => s.filters.tagIds.length > 0)
  const settingsOpen = useProtectStore((s) => s.settingsOpen)
  const openSettings = useProtectStore((s) => s.openSettings)
  const closeSettings = useProtectStore((s) => s.closeSettings)

  const section = effectiveSection(tab, ratingMenu)
  const viewOwner = viewSection(view)
  const onSection = (id: SidebarTab) => {
    if (settingsOpen) {
      closeSettings()
      showSection(id)
    } else selectSection(id)
  }

  return (
    <nav aria-label={t("rail.label")} className="flex w-(--rail-w) shrink-0 flex-col items-center gap-1 border-r border-sidebar-border bg-sidebar pb-2">
      <div className="mb-1 grid h-(--chrome-h) w-full shrink-0 place-items-center border-b border-sidebar-border">
        <RailButton label={t("rail.toggleSidebar")} aria-expanded={panelVisible} aria-controls={SIDEBAR_PANEL_ID} onClick={toggleSidebar}>
          <img src={symbol} alt="" aria-hidden="true" className={panelVisible ? "size-6.5" : "size-6.5 opacity-70"} />
        </RailButton>
      </div>
      {RAIL_SECTIONS.filter((s) => s.id !== "rating" || ratingMenu === true).map(({ id, icon }) => {
        const active = !settingsOpen && id === section
        const hasView = id === viewOwner || (id === "tags" && tagsActive)
        return (
          <RailButton
            key={id}
            icon={icon}
            label={t(`rail.section.${id}`)}
            active={active}
            dotLabel={hasView && !(active && panelVisible) ? t("rail.currentView") : null}
            aria-expanded={active && panelVisible}
            aria-controls={SIDEBAR_PANEL_ID}
            onClick={() => onSection(id)}
          />
        )
      })}
      <div className="flex-1" />
      <hr className="my-1 w-7 border-sidebar-border" />
      <RailButton
        icon={Settings}
        label={t("rail.settings")}
        active={settingsOpen}
        aria-pressed={settingsOpen}
        onClick={() => (settingsOpen ? closeSettings() : openSettings())}
      />
    </nav>
  )
}
