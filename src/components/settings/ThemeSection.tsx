import { useState } from "react"
import { useTranslation } from "react-i18next"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { useUpdateSettings } from "@/hooks/useSettings"
import { errorKey } from "@/lib/errors"
import type { Settings, ThemeSetting } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { resolveTheme, type ResolvedTheme } from "@/theme/resolve"
import { useOsDark } from "@/theme/useOsDark"
import { SubsectionHeading } from "./SettingsSubsection"

type Mode = ThemeSetting["mode"]
type SystemDark = ThemeSetting["systemDark"]

const MODES: Mode[] = ["system", "light", "gray", "dim", "dark"]
const SYSTEM_DARKS: SystemDark[] = ["dim", "dark"]

/** 미리보기 범위 클래스 — 토큰이 CSS 변수라 범위 요소에 테마 클래스를 주면 실제 값으로 그려진다(.light 는 index.css 에서 :root 값을 되돌린다). */
function scopeProps(theme: ResolvedTheme) {
  if (theme === "light") return { className: "light" }
  if (theme === "gray") return { className: "light", "data-theme": theme }
  return { className: "dark", "data-theme": theme }
}

/** 한 테마의 축소 화면: 사이드바 + 카드 격자. 장식이므로 스크린리더에서 숨긴다. */
function MiniPreview({ theme, className }: { theme: ResolvedTheme; className?: string }) {
  const { className: scope, ...attrs } = scopeProps(theme)
  return (
    <div {...attrs} className={cn(scope, "flex h-full min-w-0 flex-1 bg-background", className)}>
      <div className="w-1/4 shrink-0 border-r border-sidebar-border bg-sidebar p-1">
        <div className="h-1 rounded-sm bg-sidebar-accent" />
      </div>
      <div className="grid flex-1 grid-cols-2 content-start gap-1 p-1">
        <div className="h-3 rounded-sm bg-muted" />
        <div className="h-3 rounded-sm bg-muted" />
        <div className="col-span-2 h-1 rounded-sm bg-muted-foreground/60" />
      </div>
    </div>
  )
}

function Preview({ mode, systemDark }: { mode: Mode; systemDark: SystemDark }) {
  return (
    <div aria-hidden="true" className="flex h-14 overflow-hidden rounded-md border border-border">
      {mode === "system" ? (
        <>
          <MiniPreview theme="light" />
          <MiniPreview theme={systemDark} />
        </>
      ) : (
        <MiniPreview theme={mode} />
      )}
    </div>
  )
}

/** 설정 > 일반 > "화면 테마": 카드형 4선택 + (시스템일 때만) Windows 다크일 때의 딤/다크 선택. 선택 즉시 저장한다. */
export function ThemeSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const osDark = useOsDark()
  const [error, setError] = useState<string | null>(null)
  const { mode, systemDark } = settings.theme

  // 항상 두 값을 함께 보낸다 — 시스템이 아닌 테마로 가도 systemDark 는 유지된다
  const save = (next: ThemeSetting) => {
    setError(null)
    update.mutate({ theme: next }, { onError: (e) => setError(t(errorKey(e))) })
  }

  const applied = resolveTheme(settings.theme, osDark)

  return (
    <section aria-label={t("settings.general.theme.label")} className="flex flex-col gap-3">
      <SubsectionHeading title={t("settings.general.theme.label")} description={t("settings.general.theme.hint")} />
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <RadioGroup
        aria-label={t("settings.general.theme.label")}
        value={mode}
        onValueChange={(v) => save({ mode: v as Mode, systemDark })}
        className="grid-cols-2 gap-2 lg:grid-cols-5"
      >
        {MODES.map((m) => (
          <label
            key={m}
            htmlFor={`theme-${m}`}
            className="flex cursor-pointer flex-col gap-2 rounded-lg border border-border bg-card p-2 has-[[aria-checked=true]]:border-primary has-[[aria-checked=true]]:ring-1 has-[[aria-checked=true]]:ring-primary has-[:focus-visible]:ring-3 has-[:focus-visible]:ring-ring/50"
          >
            <Preview mode={m} systemDark={systemDark} />
            <div className="flex items-start gap-2">
              <RadioGroupItem id={`theme-${m}`} value={m} aria-label={t(`settings.general.theme.${m}`)} className="mt-0.5" />
              <div className="flex min-w-0 flex-col">
                <span className="text-sm font-medium break-keep">{t(`settings.general.theme.${m}`)}</span>
                <span className="text-xs text-muted-foreground break-keep">{t(`settings.general.theme.${m}Hint`)}</span>
              </div>
            </div>
          </label>
        ))}
      </RadioGroup>
      {mode === "system" && (
        <div className="flex flex-col gap-2 rounded-lg border border-border bg-muted/40 p-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span id="theme-system-dark-label" className="text-sm font-medium break-keep">
              {t("settings.general.theme.systemDarkLabel")}
            </span>
            <RadioGroup
              aria-labelledby="theme-system-dark-label"
              value={systemDark}
              onValueChange={(v) => save({ mode, systemDark: v as SystemDark })}
              className="flex w-auto gap-4"
            >
              {SYSTEM_DARKS.map((d) => (
                <label key={d} htmlFor={`theme-system-dark-${d}`} className="flex cursor-pointer items-center gap-2 text-sm">
                  <RadioGroupItem id={`theme-system-dark-${d}`} value={d} />
                  {t(d === "dim" ? "settings.general.theme.systemDarkDim" : "settings.general.theme.systemDarkDark")}
                </label>
              ))}
            </RadioGroup>
          </div>
          <p role="status" className="text-xs text-muted-foreground break-keep">
            {t("settings.general.theme.status", {
              os: t(osDark ? "settings.general.theme.osModeDark" : "settings.general.theme.osModeLight"),
              applied: t(`settings.general.theme.${applied}`),
            })}
          </p>
        </div>
      )}
    </section>
  )
}
