import { useQuery } from "@tanstack/react-query"
import { ShieldCheck } from "lucide-react"
import { useTranslation } from "react-i18next"
import { PasswordForm } from "@/components/protect/PasswordForm"
import { Separator } from "@/components/ui/separator"
import { Switch } from "@/components/ui/switch"
import { qk } from "@/hooks/keys"
import { useProtectedAction } from "@/hooks/useProtectedAction"
import { useUpdateSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import type { Settings } from "@/lib/ipc-contract"
import { DangerZone } from "./DangerZone"
import { NumberField } from "./NumberField"
import { SettingRow } from "./SettingRow"

export function ProtectionTab({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const api = useApi()
  const update = useUpdateSettings()
  const protect = useProtectedAction()
  const status = useQuery({ queryKey: qk.protection, queryFn: () => api.getProtectionStatus() })

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-muted-foreground break-keep">{t("settings.protection.description")}</p>
      {status.data && (
        <section aria-label={t("settings.protection.password")} className="flex flex-col gap-3">
          <h3 className="flex items-center gap-1.5 text-sm font-medium">
            <ShieldCheck aria-hidden="true" className="size-4" />
            {t(status.data.passwordSet ? "settings.protection.change" : "settings.protection.set")}
          </h3>
          <PasswordForm key={String(status.data.passwordSet)} mode={status.data.passwordSet ? "change" : "set"} />
        </section>
      )}
      <Separator />
      <NumberField
        label={t("settings.protection.exempt")}
        unit={t("settings.unit.minutes")}
        value={settings.authExemptMinutes}
        min={0}
        max={60}
        onCommit={async (authExemptMinutes) => {
          // 올리는 변경은 보호 명령 — 면제 중이 아니면 비밀번호 확인 후 저장 (내리는 변경은 바로 저장)
          if (authExemptMinutes <= settings.authExemptMinutes || !status.data?.passwordSet) {
            return update.mutateAsync({ authExemptMinutes })
          }
          await protect(() => update.mutateAsync({ authExemptMinutes }))
        }}
      />
      <p className="text-xs text-muted-foreground break-keep">{t("settings.protection.exemptHint")}</p>
      <DangerZone>
        <SettingRow label={t("settings.protection.originalActions")} description={t("settings.protection.originalActionsHint")}>
          <Switch
            aria-label={t("settings.protection.originalActions")}
            checked={settings.originalActionsEnabled}
            onCheckedChange={(originalActionsEnabled) => update.mutate({ originalActionsEnabled })}
          />
        </SettingRow>
      </DangerZone>
    </div>
  )
}
