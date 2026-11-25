import { ArrowLeft } from "lucide-react"
import { useEffect } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useSettings } from "@/hooks/useSettings"
import { OPEN_LAYER } from "@/lib/openLayer"
import { useProtectStore } from "@/stores"
import { SettingsNav } from "./SettingsNav"
import { SETTINGS_MENU } from "./settingsMenu"


/** 설정 전체 화면 — 상단 바(← 라이브러리로) / 사이드 메뉴(nav) / 본문(main, 유일한 세로 스크롤 영역). 상태 표시줄은 AppShell 이 아래에 유지한다. */
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
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <Button variant="ghost" size="sm" onClick={close}>
          <ArrowLeft aria-hidden="true" />
          {t("settings.backToLibrary")}
        </Button>
        <h1 className="text-sm font-semibold">{t("settings.title")}</h1>
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
