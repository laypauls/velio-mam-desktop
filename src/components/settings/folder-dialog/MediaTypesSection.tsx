import { useId } from "react"
import { useTranslation } from "react-i18next"
import { MEDIA_TYPE_ORDER, MediaTypeFields } from "@/components/folders/MediaTypeFields"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { useFolderAssetCounts } from "@/hooks/useFolderAdmin"
import { formatNumber } from "@/lib/format"
import type { MediaType, MediaTypeChangeMode, MediaTypeSet } from "@/lib/ipc-contract"

interface Props {
  folderId: number
  value: MediaTypeSet
  onChange: (next: MediaTypeSet) => void
  /** 이번 편집으로 제외되는 유형 */
  excluded: MediaType[]
  mode: MediaTypeChangeMode
  onModeChange: (mode: MediaTypeChangeMode) => void
  disabled: boolean
  /** 인라인 오류 — 하나도 선택하지 않았거나 저장 실패 */
  error: string | null
}

/** 스캔 대상 유형 — 제외되는 유형이 있을 때만 기존 항목 유지/정리 선택지를 보인다. */
export function MediaTypesSection({ folderId, value, onChange, excluded, mode, onModeChange, disabled, error }: Props) {
  const { t } = useTranslation()
  const radioId = useId()
  const counts = useFolderAssetCounts(folderId)
  const excludedText = MEDIA_TYPE_ORDER.filter((k) => excluded.includes(k))
    .map((k) =>
      counts.data
        ? t("settings.folders.typesExcludedItem", { type: t(`asset.type.${k}`), count: formatNumber(counts.data[k]) })
        : t(`asset.type.${k}`),
    )
    .join(", ")

  return (
    <div className="flex flex-col gap-3">
      <MediaTypeFields value={value} counts={counts.data} onChange={onChange} disabled={disabled} error={error} />
      {excluded.length > 0 && (
        <div className="flex flex-col gap-2">
          <p className="text-sm font-medium break-keep">{t("settings.folders.typesExcluded", { list: excludedText })}</p>
          <RadioGroup value={mode} onValueChange={(v) => onModeChange(v as MediaTypeChangeMode)} disabled={disabled}>
            <div className="flex items-start gap-2">
              <RadioGroupItem id={`${radioId}-keep`} value="keep" className="mt-0.5" />
              <Label htmlFor={`${radioId}-keep`} className="!items-start break-keep">
                {t("settings.folders.typesKeep")}
              </Label>
            </div>
            <div className="flex items-start gap-2">
              <RadioGroupItem id={`${radioId}-purge`} value="purge" className="mt-0.5" aria-describedby={`${radioId}-purge-hint`} />
              <div className="flex flex-col gap-1">
                <Label htmlFor={`${radioId}-purge`} className="!items-start break-keep">
                  {t("settings.folders.typesPurge")}
                </Label>
                <p id={`${radioId}-purge-hint`} className="text-xs text-muted-foreground break-keep">
                  {t("settings.folders.typesPurgeHint")}
                </p>
                <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.typesPurgeWarning")}</p>
              </div>
            </div>
          </RadioGroup>
        </div>
      )}
    </div>
  )
}
