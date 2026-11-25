import { useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useRemoteSources } from "@/hooks/useRemoteSources"
import type { RemoteSourceKind } from "@/lib/ipc-contract"
import { RemoteSourcesDialog } from "../remote/RemoteSourcesDialog"

interface Props {
  label: string
  /** 이 감지기가 받는 접속 종류 — 목록을 이 종류로 거른다 */
  kind: RemoteSourceKind
  /** 고른 접속 ID 문자열 ("" = 아직 안 고름) */
  value: string
  error?: string
  onChange: (value: string) => void
}

/** 원격 접속 선택 + [접속 관리]. 방금 만든 접속은 바로 고른다. */
export function RemoteSourceField({ label, kind, value, error, onChange }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const errorId = `${id}-error`
  const sources = useRemoteSources()
  const [manage, setManage] = useState(false)
  const list = (sources.data ?? []).filter((s) => s.kind === kind)
  // 고른 접속이 목록에 없으면(삭제됨·다른 종류) 그 사실을 보여 준다 — 조용히 비우지 않는다
  const missing = value !== "" && sources.data !== undefined && !list.some((s) => String(s.sourceId) === value)

  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={id} className="!items-start">
        {label}
      </Label>
      <div className="flex items-center gap-2">
        <Select value={value} onValueChange={onChange}>
          <SelectTrigger id={id} className="min-w-0 flex-1" aria-invalid={error ? true : undefined} aria-describedby={error ? errorId : undefined}>
            <SelectValue placeholder={t("settings.folders.detection.remoteSourcePlaceholder")} />
          </SelectTrigger>
          <SelectContent>
            {list.map((s) => (
              <SelectItem key={s.sourceId} value={String(s.sourceId)}>
                {s.name} ({s.host})
              </SelectItem>
            ))}
            {missing && <SelectItem value={value}>{t("settings.folders.detection.remoteSourceMissing", { id: value })}</SelectItem>}
          </SelectContent>
        </Select>
        <Button type="button" variant="outline" className="shrink-0" onClick={() => setManage(true)}>
          {t("settings.folders.detection.manageRemote")}
        </Button>
      </div>
      {sources.data !== undefined && list.length === 0 && (
        <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.detection.remoteSourceEmpty")}</p>
      )}
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
      <RemoteSourcesDialog open={manage} onOpenChange={setManage} initialKind={kind} onCreated={(s) => onChange(String(s.sourceId))} />
    </div>
  )
}
