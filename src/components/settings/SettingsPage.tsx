import { ArrowLeft } from "lucide-react"
import { useEffect } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { WindowControls } from "@/components/shell/WindowControls"
import { useSettings } from "@/hooks/useSettings"
import { OPEN_LAYER } from "@/lib/openLayer"
import { useProtectStore } from "@/stores"
import { SettingsNav } from "./SettingsNav"
import { SETTINGS_MENU } from "./settingsMenu"


/**
 * 설정 화면 — 상단 바(← 라이브러리로) / 사이드 메뉴(nav) / 본문(main, 유일한 세로 스크롤 영역). 레일 오른쪽 영역만 덮으며 레일·상태 표시줄은 AppShell 이 유지한다.
 * 상단 바는 TopBar 와 같은 높이의 창 이동 영역이고 오른쪽에 창 버튼을 이어받는다(드래그 속성은 헤더·래퍼·빈 칸에만, 버튼에는 없음).
 */
export function SettingsPage() {
  const { t } = useTranslation()
  const section = useProtectStore((s) => s.settingsSection)
  const setSection = useProtectStore((s) => s.setSettingsSection)
  const close = useProtectStore((s) => s.closeSettings)
  const settings = useSettings()
  const item = SETTINGS_MENU.find((m) => m.id === section) ?? SETTINGS_MENU[0]
  const Page = item.page

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return
      if (document.querySelector(OPEN_LAYER)) return
      close()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [close])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header data-tauri-drag-region className="h-(--chrome-h) shrink-0 border-b border-border">
        <div data-tauri-drag-region className="flex h-full items-center gap-2 pl-3">
          <Button variant="ghost" size="sm" onClick={close}>
            <ArrowLeft aria-hidden="true" />
            {t("settings.backToLibrary")}
          </Button>
          <h1 data-tauri-drag-region className="text-sm font-semibold">
            {t("settings.title")}
          </h1>
          <div data-tauri-drag-region className="h-full min-w-4 flex-1" />
          <WindowControls />
        </div>
      </header>
      <div className="flex min-h-0 flex-1">
        <SettingsNav active={item.id} onSelect={setSection} />
        {/* relative 필수: 스크롤 컨테이너가 위치 기준이 아니면 sr-only(caption)·숨김 select 같은 absolute 자손이 바깥 absolute 래퍼를 기준으로 잡혀 스크롤 클리핑을 벗어나 문서 스크롤이 생긴다 */}
        <main className="relative min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
          <div className="flex min-h-full max-w-3xl flex-col gap-4 px-6 py-5">
            <div className="flex flex-col gap-1">
              <h2 className="text-lg font-semibold">{t(item.labelKey)}</h2>
              <p className="text-sm text-muted-foreground break-keep">{t(`settings.page.${item.id}`)}</p>
            </div>
            {settings.isError && (
              <p role="alert" className="text-sm text-destructive">
                {t("settings.loadError")}
              </p>
            )}
            {settings.data && <Page key={item.id} settings={settings.data} />}
          </div>
        </main>
      </div>
    </div>
  )
}
