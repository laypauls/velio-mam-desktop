import { Loader2, ShieldAlert, TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import type { PluginInstallFlow } from "@/hooks/usePlugins"
import { errorKey } from "@/lib/errors"
import type { PluginPermission, PluginPreview } from "@/lib/ipc-plugin"
import { usePermissionLabel } from "./pluginPermissions"

function PermissionList({ title, permissions, highlight = false }: { title: string; permissions: PluginPermission[]; highlight?: boolean }) {
  const label = usePermissionLabel()
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <h3 className={highlight ? "text-sm font-medium text-destructive" : "text-sm font-medium"}>{title}</h3>
      <ul className="flex list-disc flex-col gap-0.5 pl-5 text-sm">
        {permissions.map((p) => (
          <li key={p}>{label(p)}</li>
        ))}
      </ul>
    </section>
  )
}

function ConsentBody({ preview }: { preview: PluginPreview }) {
  const { t } = useTranslation()
  const added = preview.change === "upgrade" ? preview.addedPermissions : []
  const kept = preview.permissions.filter((p) => !added.includes(p))
  // 같은 ID 의 설치본 또는 보존 데이터가 있으면 다른 작성자의 패키지가 기존 데이터에 접근할 수 있다
  const overwrites = preview.installed !== null || preview.hasKeptData
  return (
    <div className="flex flex-col gap-3 break-keep">
      <p className="flex items-start gap-1.5 text-sm font-medium text-destructive">
        <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
        {t("settings.plugins.unsignedWarning")}
      </p>
      {overwrites && (
        <p className="flex items-start gap-1.5 text-sm font-medium text-destructive">
          <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
          {t("settings.plugins.consent.overwrite")}
        </p>
      )}
      {preview.description && <p className="text-sm text-muted-foreground">{preview.description}</p>}
      {preview.permissions.length === 0 && <p className="text-sm">{t("settings.plugins.consent.noPermissions")}</p>}
      {added.length > 0 && <PermissionList highlight title={t("settings.plugins.consent.added")} permissions={added} />}
      {kept.length > 0 && (
        <PermissionList title={t(added.length > 0 ? "settings.plugins.consent.kept" : "settings.plugins.consent.requested")} permissions={kept} />
      )}
      <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
        <ShieldAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        {t("settings.plugins.consent.notProvided")}
      </p>
    </div>
  )
}

/** 설치·업그레이드 권한 동의 — 검사 실패도 같은 Dialog 안에 인라인으로 보여 주고(닫기만), 설치 실패는 열어 둔다. */
export function PluginConsentDialog({ flow }: { flow: PluginInstallFlow }) {
  const { t } = useTranslation()
  const { session, installing, installError } = flow
  const preview = session?.preview ?? null
  const error = installError ?? session?.error ?? null
  return (
    <Dialog open={session !== null} onOpenChange={(o) => !o && !installing && flow.close()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {preview
              ? t(`settings.plugins.consent.title.${preview.change}`, { name: preview.displayName, version: preview.version })
              : t("settings.plugins.consent.inspectFailedTitle")}
          </DialogTitle>
          <DialogDescription className="sr-only">{t("settings.plugins.consent.description")}</DialogDescription>
        </DialogHeader>
        {preview && <ConsentBody preview={preview} />}
        {error !== null && (
          <p role="alert" className="text-sm text-destructive break-keep">
            {t(errorKey(error))}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={installing} onClick={flow.close}>
            {t(preview ? "common.cancel" : "common.close")}
          </Button>
          {preview && (
            <Button aria-busy={installing} disabled={installing} onClick={flow.confirm}>
              {installing && <Loader2 aria-hidden="true" className="animate-spin" />}
              {t("settings.plugins.consent.install")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
