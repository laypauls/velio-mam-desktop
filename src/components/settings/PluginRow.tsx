import { Play, RotateCcw, Trash2, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { useRetryPluginFailed, useRunAllPlugin, useSetPluginEnabled } from "@/hooks/usePlugins"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { PluginInfo, PluginTaskCount } from "@/lib/ipc-plugin"
import { usePermissionLabel } from "./pluginPermissions"

/** 전체 실행 대상 — 내보내기 제공자는 대기열 작업이 아니라 선택 자산 내보내기로만 쓴다 */
const runnableProviders = (plugin: PluginInfo) => plugin.providers.filter((p) => p.kind !== "export")

/** 설치된 플러그인 한 카드 — 제목·버전·미서명·ID·권한·활성 스위치 / 작업 수·재시도·전체 실행·제거. 작업 실패는 카드 안 인라인. */
export function PluginRow({ plugin, counts, onUninstall }: { plugin: PluginInfo; counts: PluginTaskCount | undefined; onUninstall: () => void }) {
  const { t } = useTranslation()
  const permissionLabel = usePermissionLabel()
  const setEnabled = useSetPluginEnabled()
  const runAll = useRunAllPlugin()
  const retry = useRetryPluginFailed()
  const [error, setError] = useState<unknown>(null)
  const usable = plugin.status === "ok"
  const pending = counts?.pending ?? 0
  const failed = counts?.failed ?? 0
  const providers = runnableProviders(plugin)
  const granted = plugin.permissions.filter((p) => plugin.granted.includes(p))
  const track = { onMutate: () => setError(null), onError: (e: unknown) => setError(e) }

  return (
    <li className="flex flex-col gap-2 rounded-xl border border-border p-4">
      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <h3 className="min-w-0 text-sm font-semibold break-all">{plugin.displayName}</h3>
          <span className="text-xs text-muted-foreground">v{plugin.version}</span>
          {plugin.unsigned && (
            <Badge variant="destructive" className="rounded-sm">
              <TriangleAlert aria-hidden="true" />
              {t("settings.plugins.unsigned")}
            </Badge>
          )}
          {plugin.status !== "ok" && (
            <Badge variant="outline" className="rounded-sm">
              {t(`settings.plugins.status.${plugin.status}`)}
            </Badge>
          )}
        </div>
        <Switch
          checked={plugin.enabled}
          disabled={!usable || setEnabled.isPending}
          aria-label={t("settings.plugins.enableLabel", { name: plugin.displayName })}
          onCheckedChange={(enabled) => setEnabled.mutate({ pluginId: plugin.id, enabled }, track)}
        />
      </div>
      <p className="text-xs break-all text-muted-foreground">{plugin.id}</p>
      {plugin.description && <p className="text-xs break-keep text-muted-foreground">{plugin.description}</p>}
      <p className="text-xs break-keep">
        {t("settings.plugins.permissions", { list: granted.length > 0 ? granted.map(permissionLabel).join(" · ") : t("settings.plugins.permissionsNone") })}
      </p>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-xs">
          <span>{t("settings.plugins.counts", { pending: formatNumber(pending), failed: formatNumber(failed) })}</span>
          {failed > 0 && (
            <Button size="xs" variant="outline" disabled={retry.isPending} onClick={() => retry.mutate(plugin.id, track)}>
              <RotateCcw aria-hidden="true" />
              {t("settings.plugins.retry")}
            </Button>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {providers.map((provider) => (
            <Button
              key={provider.id}
              size="sm"
              variant="outline"
              disabled={!plugin.enabled || !usable || runAll.isPending}
              onClick={() => runAll.mutate({ pluginId: plugin.id, providerId: provider.id }, track)}
            >
              <Play aria-hidden="true" />
              {providers.length > 1 ? t("settings.plugins.runAllNamed", { name: provider.title }) : t("settings.plugins.runAll")}
            </Button>
          ))}
          <Button size="sm" variant="outline" onClick={onUninstall}>
            <Trash2 aria-hidden="true" />
            {t("settings.plugins.remove")}
          </Button>
        </div>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(error))}
        </p>
      )}
    </li>
  )
}
