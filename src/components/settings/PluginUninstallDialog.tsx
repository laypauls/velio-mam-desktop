import { Info, Loader2 } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { useUninstallPlugin } from "@/hooks/usePlugins"
import { errorKey } from "@/lib/errors"
import type { PluginInfo } from "@/lib/ipc-plugin"

function UninstallBody({ keepData, onKeepData, error }: { keepData: boolean; onKeepData: (v: boolean) => void; error: unknown }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-3 text-sm text-foreground break-keep">
      <p>{t("settings.plugins.uninstall.lead")}</p>
      <div className="flex items-start gap-2">
        <Checkbox id="plugin-keep-data" checked={keepData} onCheckedChange={(v) => onKeepData(v === true)} className="mt-0.5" />
        <div className="flex flex-col gap-1">
          <Label htmlFor="plugin-keep-data">{t("settings.plugins.uninstall.keepData")}</Label>
          <span className="text-xs text-muted-foreground">
            {t(keepData ? "settings.plugins.uninstall.keepDataOn" : "settings.plugins.uninstall.keepDataOff")}
          </span>
        </div>
      </div>
      <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
        <Info aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        {t("settings.plugins.uninstall.restoreNote")}
      </p>
      {error !== null && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(error))}
        </p>
      )}
    </div>
  )
}

/** 플러그인 제거 확인 — 데이터 보존 선택. 보존해도 백업 복원 시 지워진다는 안내(E1)를 항상 보여 준다. 실패는 Dialog 안 인라인. */
export function PluginUninstallDialog({ plugin, onClose }: { plugin: PluginInfo | null; onClose: () => void }) {
  const { t } = useTranslation()
  const uninstall = useUninstallPlugin()
  const [keepData, setKeepData] = useState(false)
  const close = () => {
    uninstall.reset()
    setKeepData(false)
    onClose()
  }
  return (
    <AlertDialog open={plugin !== null} onOpenChange={(o) => !o && !uninstall.isPending && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("settings.plugins.uninstall.title", { name: plugin?.displayName ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div>
              <UninstallBody keepData={keepData} onKeepData={setKeepData} error={uninstall.isError ? uninstall.error : null} />
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={uninstall.isPending}>{t("common.cancel")}</AlertDialogCancel>
          <Button
            variant="destructive"
            aria-busy={uninstall.isPending}
            disabled={uninstall.isPending || plugin === null}
            onClick={() => plugin && uninstall.mutate({ pluginId: plugin.id, keepData }, { onSuccess: close })}
          >
            {uninstall.isPending && <Loader2 aria-hidden="true" className="animate-spin" />}
            {t("settings.plugins.uninstall.confirm")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
