import { useState } from "react"
import { useTranslation } from "react-i18next"
import { FolderOpen, RefreshCw, TriangleAlert } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useLocales } from "@/hooks/useLocales"
import { useUpdateSettings } from "@/hooks/useSettings"
import { BUILTIN_LANGUAGES, DEFAULT_LANGUAGE, isBuiltinLanguage } from "@/i18n"
import { applyLanguage } from "@/i18n/applyLanguage"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { LocaleInfo, Settings } from "@/lib/ipc-contract"
import { SubsectionHeading } from "./SettingsSubsection"

/**
 * 내장 언어의 자국어 표기(endonym). 현재 표시 언어와 무관하게 고정이라 번역 키로 두지 않는다
 * (언어팩에는 한글을 넣을 수 없다 — packs.test T-ui-i18n-05).
 */
const BUILTIN_LABELS: Record<(typeof BUILTIN_LANGUAGES)[number], string> = {
  en: "English (default)",
  ko: "한국어",
}

/** 설정 > 일반 > 표시 언어. 언어 폴더의 파일을 고르면 즉시 적용하고 설정에 저장한다. */
export function LanguageSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const api = useApi()
  const update = useUpdateSettings({ silent: true })
  const locales = useLocales()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const all = locales.data?.locales ?? []
  // 내장 en·ko 는 항상 위에 둔다 — 폴더의 en/ko 파일은 덮어쓰기 용도라 별도 항목으로 만들지 않는다
  const usable = all.filter((l) => l.error === null && !isBuiltinLanguage(l.code))
  const broken = all.filter((l) => l.error !== null)
  const value =
    isBuiltinLanguage(settings.language) || usable.some((l) => l.code === settings.language) ? settings.language : DEFAULT_LANGUAGE

  const label = (l: LocaleInfo) => (l.name ? t("settings.general.language.option", { name: l.name, code: l.code }) : l.code)

  async function onChange(code: string) {
    if (code === value) return
    setError(null)
    setBusy(true)
    const restore = () => applyLanguage(value, api.readLocale)
    try {
      if (!(await applyLanguage(code, api.readLocale))) {
        setError(t("settings.general.language.applyFailed"))
        await restore()
        return
      }
      update.mutate(
        { language: code },
        {
          onError: (e) => {
            setError(t(errorKey(e)))
            void restore()
          },
        },
      )
    } finally {
      setBusy(false)
    }
  }

  async function openFolder() {
    setError(null)
    try {
      await api.openLocaleFolder()
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  const describedBy = ["language-hint", error ? "language-error" : null, broken.length > 0 ? "language-warnings" : null]
    .filter(Boolean)
    .join(" ")

  return (
    <section aria-label={t("settings.general.language.label")} className="flex flex-col gap-3">
      <SubsectionHeading title={t("settings.general.language.label")} />
      <div className="flex flex-wrap items-center gap-2">
        <Select value={value} onValueChange={(c) => void onChange(c)} disabled={busy || update.isPending}>
          <SelectTrigger
            aria-label={t("settings.general.language.select")}
            aria-describedby={describedBy}
            aria-invalid={error ? true : undefined}
            className="w-56"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {BUILTIN_LANGUAGES.map((code) => (
              <SelectItem key={code} value={code}>
                {BUILTIN_LABELS[code]}
              </SelectItem>
            ))}
            {usable.map((l) => (
              <SelectItem key={l.code} value={l.code}>
                {label(l)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button type="button" variant="outline" size="sm" onClick={() => void openFolder()}>
          <FolderOpen />
          {t("settings.general.language.openFolder")}
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={locales.isFetching} onClick={() => void locales.refetch()}>
          <RefreshCw />
          {t("settings.general.language.refresh")}
        </Button>
      </div>
      <p id="language-hint" className="text-xs text-muted-foreground break-keep">
        {t("settings.general.language.hint")}
      </p>
      {locales.isError && (
        <p role="alert" className="text-xs text-destructive break-keep">
          {t("settings.general.language.loadFailed")}
        </p>
      )}
      {error && (
        <p id="language-error" role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
      {broken.length > 0 && (
        <ul id="language-warnings" aria-label={t("settings.general.language.fileErrorList")} className="flex flex-col gap-1">
          {broken.map((l) => (
            <li key={l.code} className="flex items-start gap-1.5 text-xs text-muted-foreground break-keep">
              <TriangleAlert aria-hidden className="mt-0.5 size-3.5 shrink-0" />
              <span>
                {t("settings.general.language.fileError", {
                  code: l.code,
                  reason: t(`settings.general.language.reasons.${l.error ?? "read_failed"}`),
                })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
