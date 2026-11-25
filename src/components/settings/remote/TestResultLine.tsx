import { CircleCheck, ShieldAlert, TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import type { RemoteSourceKind, RemoteTestResult } from "@/lib/ipc-contract"

interface Props {
  result: RemoteTestResult
  kind: RemoteSourceKind
  /** 저장된(확인한) 지문 — 서버 지문과 다르면 경고한다 */
  hostKey: string | null
  disabled?: boolean
  onConfirm: () => void
}

/** [접속 시험] 결과 한 줄. 지문 확인이 필요하면 지문과 확인 버튼을 함께 보여 준다 (확인은 사용자의 명시적 동작). */
export function TestResultLine({ result, kind, hostKey, disabled, onConfirm }: Props) {
  const { t } = useTranslation()
  if (result.ok) {
    const detail =
      kind === "ses_queue"
        ? t("settings.folders.remote.testOkQueue", { rows: result.rowCount ?? 0, maxId: result.maxId ?? 0 })
        : t("settings.folders.remote.testOkFiles", { count: result.fileCount ?? 0 })
    return (
      <p role="status" className="flex items-start gap-1.5 text-xs break-keep">
        <CircleCheck aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-primary" />
        <span>{t("settings.folders.remote.testOk", { detail })}</span>
      </p>
    )
  }
  const key = result.errorKey ?? "exec"
  const needsConfirm = (key === "host_key_unconfirmed" || key === "host_key_mismatch") && result.fingerprint !== null
  return (
    <div role="alert" className="flex flex-col gap-2 text-xs break-keep">
      <p className="flex items-start gap-1.5 text-destructive">
        <TriangleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        <span>{t(`remote.error.${key}`, { defaultValue: t("remote.error.exec") })}</span>
      </p>
      {needsConfirm && (
        <div className="flex flex-col gap-1.5 rounded-lg border border-border p-3">
          <p className="flex items-start gap-1.5 font-medium">
            <ShieldAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
            <span>{t(key === "host_key_mismatch" ? "settings.folders.remote.hostKeyChanged" : "settings.folders.remote.hostKeyFirst")}</span>
          </p>
          {key === "host_key_mismatch" && hostKey !== null && (
            <p className="text-muted-foreground">
              {t("settings.folders.remote.hostKeyStored")} <code className="break-all font-mono">{hostKey}</code>
            </p>
          )}
          <p>
            {t("settings.folders.remote.hostKeyOffered")} <code className="break-all font-mono">{result.fingerprint}</code>
          </p>
          <div>
            <Button type="button" size="sm" variant="outline" disabled={disabled} onClick={onConfirm}>
              {t(key === "host_key_mismatch" ? "settings.folders.remote.hostKeyReplace" : "settings.folders.remote.hostKeyTrust")}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}
