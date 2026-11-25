import { Pencil, ServerCog, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { RemoteSource } from "@/lib/ipc-contract"

interface Props {
  sources: RemoteSource[]
  onEdit: (source: RemoteSource) => void
  onDelete: (source: RemoteSource) => void
}

/** 저장된 접속 목록 — 이름·종류·호스트와 지문 확인 여부. 비밀은 표시하지 않는다. */
export function RemoteSourceList({ sources, onEdit, onDelete }: Props) {
  const { t } = useTranslation()
  if (sources.length === 0) {
    return (
      <div className="flex flex-col items-center gap-1.5 rounded-lg border border-dashed border-border p-6 text-center">
        <ServerCog aria-hidden="true" className="size-6 text-muted-foreground" />
        <p className="text-sm font-medium">{t("settings.folders.remote.emptyTitle")}</p>
        <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.remote.emptyDescription")}</p>
      </div>
    )
  }
  return (
    <ul aria-label={t("settings.folders.remote.listLabel")} className="flex flex-col divide-y divide-border rounded-lg border border-border">
      {sources.map((s) => (
        <li key={s.sourceId} className="flex items-center gap-2 px-3 py-2">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <div className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-sm font-medium" title={s.name}>
                {s.name}
              </span>
              <Badge variant="outline" className="shrink-0 rounded-sm">
                {t(`settings.folders.remote.kinds.${s.kind}`)}
              </Badge>
              {s.hostKey === null && (
                <Badge variant="secondary" className="shrink-0 rounded-sm">
                  {t("settings.folders.remote.hostKeyUnconfirmedBadge")}
                </Badge>
              )}
            </div>
            <span className="truncate text-xs text-muted-foreground" title={`${s.user}@${s.host}:${s.port}`}>
              {s.user}@{s.host}:{s.port}
            </span>
          </div>
          <Button size="icon-sm" variant="ghost" aria-label={t("settings.folders.remote.edit", { name: s.name })} onClick={() => onEdit(s)}>
            <Pencil aria-hidden="true" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={t("settings.folders.remote.deleteLabel", { name: s.name })} onClick={() => onDelete(s)}>
            <Trash2 aria-hidden="true" />
          </Button>
        </li>
      ))}
    </ul>
  )
}
