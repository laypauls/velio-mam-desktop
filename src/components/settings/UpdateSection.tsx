import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useUpdater } from "@/hooks/useUpdater"
import { updateErrorKey } from "@/lib/update-errors"
import { useUpdateStore } from "@/stores/update"
import { SettingsSubsection } from "./SettingsSubsection"

/** 설정 > 일반: 수동 업데이트 확인. 실패는 인라인으로 표시한다. */
export function UpdateSection() {
  const { t } = useTranslation()
  const { check, open } = useUpdater()
  const checking = useUpdateStore((s) => s.checking)
  const version = useUpdateStore((s) => s.available?.version)
  const upToDate = useUpdateStore((s) => s.upToDate)
  const error = useUpdateStore((s) => s.checkError)
  return (
    <SettingsSubsection title={t("update.section")}>
      <div className="flex items-center gap-2">
        <Button size="sm" variant="outline" disabled={checking} onClick={() => void check(true)}>
          {checking ? t("update.checking") : t("update.check")}
        </Button>
        {version && (
          <Button size="sm" variant="link" onClick={open}>
            {t("update.availableLink", { version })}
          </Button>
        )}
        {upToDate && !error && <span className="text-sm text-muted-foreground">{t("update.upToDate")}</span>}
      </div>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {t(updateErrorKey(error))}
        </p>
      )}
    </SettingsSubsection>
  )
}
