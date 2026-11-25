import { useRef, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import type { SettingsSection } from "@/stores"
import { SETTINGS_MENU } from "./settingsMenu"

/**
 * 설정 사이드 메뉴 — 현재 항목은 `aria-current="page"`, 위·아래 화살표로 이동(Home/End 포함).
 * 좁은 창(md 미만)에서는 아이콘만 보이고 라벨은 스크린리더·툴팁(title)으로 남는다.
 */
export function SettingsNav({ active, onSelect }: { active: SettingsSection; onSelect: (id: SettingsSection) => void }) {
  const { t } = useTranslation()
  const refs = useRef(new Map<SettingsSection, HTMLButtonElement>())

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const last = SETTINGS_MENU.length - 1
    const cur = SETTINGS_MENU.findIndex((m) => m.id === active)
    let next: number
    if (e.key === "ArrowDown") next = cur >= last ? 0 : cur + 1
    else if (e.key === "ArrowUp") next = cur <= 0 ? last : cur - 1
    else if (e.key === "Home") next = 0
    else if (e.key === "End") next = last
    else return
    e.preventDefault()
    const id = SETTINGS_MENU[next].id
    onSelect(id)
    refs.current.get(id)?.focus()
  }

  return (
    // relative: 좁은 창의 sr-only(absolute) 라벨이 스크롤 클리핑을 벗어나지 않게 위치 기준을 만든다
    <nav aria-label={t("settings.menuLabel")} onKeyDown={onKeyDown} className="relative flex w-12 shrink-0 flex-col gap-1 overflow-y-auto border-r border-border p-2 md:w-52">
      {SETTINGS_MENU.map(({ id, labelKey, icon: Icon }) => {
        const isActive = id === active
        return (
          <button
            key={id}
            type="button"
            ref={(el) => {
              if (el) refs.current.set(id, el)
              else refs.current.delete(id)
            }}
            title={t(labelKey)}
            aria-current={isActive ? "page" : undefined}
            onClick={() => onSelect(id)}
            className={cn(
              "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              isActive ? "bg-accent font-medium text-accent-foreground" : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"
            )}
          >
            <Icon aria-hidden="true" className="size-4 shrink-0" />
            <span className="min-w-0 truncate max-md:sr-only">{t(labelKey)}</span>
          </button>
        )
      })}
    </nav>
  )
}
