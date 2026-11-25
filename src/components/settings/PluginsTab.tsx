import { FolderPlus, Loader2, Puzzle } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { usePluginCounts, usePluginInstall, usePluginList } from "@/hooks/usePlugins"
import type { PluginInfo } from "@/lib/ipc-plugin"
import { PluginConsentDialog } from "./PluginConsentDialog"
import { PluginRow } from "./PluginRow"
import { PluginUninstallDialog } from "./PluginUninstallDialog"

/** 설정 > 플러그인 — 설치 목록(활성·작업 수·전체 실행·제거)과 폴더에서 설치. 설치·업그레이드는 권한 동의 Dialog 를 거친다. */
export function PluginsTab() {
  const { t } = useTranslation()
  const list = usePluginList()
  const counts = usePluginCounts()
  const install = usePluginInstall()
  const [removing, setRemoving] = useState<PluginInfo | null>(null)
  const plugins = list.data ?? []

  const installButton = (
    <Button size="sm" variant="outline" aria-busy={install.picking} disabled={install.picking} onClick={() => void install.start()}>
      {install.picking ? <Loader2 aria-hidden="true" className="animate-spin" /> : <FolderPlus aria-hidden="true" />}
      {t("settings.plugins.install")}
    </Button>
  )

  return (
    <div className="flex flex-col gap-4">
      {list.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {t("settings.plugins.loadError")}
        </p>
      ) : list.isPending ? (
        <p role="status" aria-busy="true" className="text-sm text-muted-foreground">
          {t("common.loading")}
        </p>
      ) : plugins.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-border p-8 text-center">
          <Puzzle aria-hidden="true" className="size-10 text-muted-foreground" />
          <h3 className="text-base font-medium break-keep">{t("settings.plugins.emptyTitle")}</h3>
          <p className="max-w-sm text-sm break-keep text-muted-foreground">{t("settings.plugins.emptyDescription")}</p>
          {installButton}
        </div>
      ) : (
        <>
          <div className="flex justify-end">{installButton}</div>
          <ul aria-label={t("settings.plugins.listLabel")} className="flex flex-col gap-3">
            {plugins.map((plugin) => (
              <PluginRow
                key={plugin.id}
                plugin={plugin}
                counts={counts.data?.find((c) => c.pluginId === plugin.id)}
                onUninstall={() => setRemoving(plugin)}
              />
            ))}
          </ul>
        </>
      )}
      <PluginConsentDialog flow={install} />
      <PluginUninstallDialog plugin={removing} onClose={() => setRemoving(null)} />
    </div>
  )
}
