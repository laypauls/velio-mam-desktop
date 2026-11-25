import { Info } from "lucide-react"
import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import type { ChangeDetectorInfo, ManagedFolder } from "@/lib/ipc-contract"
import { DetectorOptionFields } from "./DetectorOptionFields"
import { WatchHealthLine } from "./WatchHealthLine"

export const WATCH_AUTO = "__auto__"

interface Props {
  folder: ManagedFolder
  /** 이 폴더 기준 감지기 목록 (불러오는 중이면 undefined) */
  detectors: ChangeDetectorInfo[] | undefined
  mode: string
  onModeChange: (mode: string) => void
  optionValues: Record<string, string>
  optionErrors: Record<string, string>
  onOptionChange: (key: string, value: string) => void
}

/** 감지기 표시 이름 — 번역이 없는 (새·제거된) 감지기는 ID 그대로. */
export function detectorName(t: (k: string, o?: Record<string, unknown>) => string, id: string): string {
  return t(`watch.detector.${id}.name`, { defaultValue: id })
}

/** 선택 목록용 이름 — 실험 감지기(`experimental`)는 "이름 (실험)". */
function detectorLabel(t: (k: string, o?: Record<string, unknown>) => string, d: ChangeDetectorInfo): string {
  const name = detectorName(t, d.id)
  return d.experimental ? t("watch.experimentalName", { name }) : name
}

/** "현재 동작" 한 줄 — 저장된 상태(`watchActive`/`watchFallbackKey`) 기준. */
export function currentBehavior(t: (k: string, o?: Record<string, unknown>) => string, folder: ManagedFolder): string {
  const name = folder.watchActive === null ? t("watch.notWatching") : detectorName(t, folder.watchActive)
  if (folder.watchFallbackKey === null) return t("settings.folders.detection.watchCurrent", { name })
  const reason = t(`watch.reason.${folder.watchFallbackKey}`, { defaultValue: folder.watchFallbackKey })
  return t("settings.folders.detection.watchCurrentReason", { name, reason })
}

/** 변경 감지 방식 선택 + 현재 동작 + 감지기별 옵션. */
export function WatchModeSection({ folder, detectors, mode, onModeChange, optionValues, optionErrors, onOptionChange }: Props) {
  const { t } = useTranslation()
  const selectId = useId()
  const list = detectors ?? []
  // 저장된 방식이 이 버전에 없으면(구버전 DB·제거된 감지기) 목록에 없는 항목으로 보여 준다
  const storedUnknown = folder.watchMode !== null && !list.some((d) => d.id === folder.watchMode) ? folder.watchMode : null
  const selected = list.find((d) => d.id === mode)

  return (
    <section className="flex flex-col gap-3" aria-labelledby={`${selectId}-title`}>
      <h4 id={`${selectId}-title`} className="text-sm font-semibold">
        {t("settings.folders.detection.watchSection")}
      </h4>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={selectId} className="!items-start">
          {t("settings.folders.detection.watchMode")}
        </Label>
        <Select value={mode} onValueChange={onModeChange}>
          <SelectTrigger id={selectId} className="w-full min-w-0">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={WATCH_AUTO}>{t("settings.folders.detection.watchAuto")}</SelectItem>
            {list.map((d) => (
              <SelectItem key={d.id} value={d.id} disabled={!d.available}>
                {d.available ? detectorLabel(t, d) : t("settings.folders.detection.watchUnavailable", { name: detectorLabel(t, d) })}
                {!d.available && d.reasonKey !== null && (
                  <span className="block text-xs text-muted-foreground break-keep">
                    {t(`watch.reason.${d.reasonKey}`, { defaultValue: d.reasonKey })}
                  </span>
                )}
              </SelectItem>
            ))}
            {storedUnknown !== null && <SelectItem value={storedUnknown}>{detectorName(t, storedUnknown)}</SelectItem>}
          </SelectContent>
        </Select>
        {selected && (
          <p className="text-xs text-muted-foreground break-keep">
            {t(`watch.detector.${selected.id}.description`, { defaultValue: "" })}
          </p>
        )}
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground break-keep">
          <Info aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
          <span>{currentBehavior(t, folder)}</span>
        </p>
        <WatchHealthLine folder={folder} />
      </div>
      {selected && (
        <DetectorOptionFields
          detectorId={selected.id}
          fields={selected.optionFields}
          values={optionValues}
          errors={optionErrors}
          onChange={onOptionChange}
        />
      )}
    </section>
  )
}
