import { Plus, X } from "lucide-react"
import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import type { PathMapping } from "@/lib/ipc-contract"

interface Props {
  rows: PathMapping[]
  onChange: (rows: PathMapping[]) => void
  /** 이미 번역된 오류 문구 */
  error?: string
  disabled?: boolean
}

const MAX_ROWS = 32

/** 경로 변환 규칙 — 스토리지(원격) 경로 → 이 PC 경로 줄 목록. 최소 1줄. */
export function PathMapRows({ rows, onChange, error, disabled }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const errorId = `${id}-error`
  const update = (i: number, patch: Partial<PathMapping>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)))
  return (
    <fieldset className="flex flex-col gap-2" aria-describedby={error ? errorId : undefined}>
      <legend className="mb-1 text-sm font-medium">{t("settings.folders.remote.pathMap")}</legend>
      <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.remote.pathMapHint")}</p>
      {rows.map((row, i) => (
        <div key={i} className="flex items-center gap-2">
          <Input
            aria-label={t("settings.folders.remote.pathMapFrom", { n: i + 1 })}
            aria-invalid={error ? true : undefined}
            className="min-w-0 flex-1 font-mono text-xs"
            placeholder="/data/incoming"
            value={row.from}
            disabled={disabled}
            onChange={(e) => update(i, { from: e.target.value })}
          />
          <span aria-hidden="true" className="text-muted-foreground">
            →
          </span>
          <Input
            aria-label={t("settings.folders.remote.pathMapTo", { n: i + 1 })}
            aria-invalid={error ? true : undefined}
            className="min-w-0 flex-1 font-mono text-xs"
            placeholder={"\\\\nas\\media"}
            value={row.to}
            disabled={disabled}
            onChange={(e) => update(i, { to: e.target.value })}
          />
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={t("settings.folders.remote.pathMapRemove", { n: i + 1 })}
            disabled={disabled || rows.length <= 1}
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
          >
            <X aria-hidden="true" />
          </Button>
        </div>
      ))}
      <div>
        <Button type="button" size="sm" variant="outline" disabled={disabled || rows.length >= MAX_ROWS} onClick={() => onChange([...rows, { from: "", to: "" }])}>
          <Plus aria-hidden="true" />
          {t("settings.folders.remote.pathMapAdd")}
        </Button>
      </div>
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </fieldset>
  )
}
