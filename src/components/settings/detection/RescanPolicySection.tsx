import { TriangleAlert } from "lucide-react"
import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { formatDateTime } from "@/lib/format"
import { KNOWN_TRIGGER_KINDS, type ManagedFolder } from "@/lib/ipc-contract"
import {
  INTERVAL_HOURS,
  POLICY_DEFAULT,
  POLICY_UNKNOWN,
  storedKind,
  windowVisible,
  type DraftErrors,
  type DraftKind,
  type PolicyDraft,
} from "@/lib/rescan-policy"
import { TimeField } from "./TimeField"
import { TRIGGER_FIELDS } from "./trigger-fields"

const KIND_LABEL: Record<(typeof KNOWN_TRIGGER_KINDS)[number], string> = {
  interval: "settings.folders.detection.policyInterval",
  schedule: "settings.folders.detection.policySchedule",
  manual: "settings.folders.detection.policyManual",
}

interface Props {
  folder: ManagedFolder
  draft: PolicyDraft
  /** 폴더에 저장돼 있던 정책이 알 수 없는 종류인가 — 그때만 "알 수 없는 정책" 항목을 보여 준다 */
  storedUnknown: boolean
  errors: DraftErrors
  defaultHours: number
  /** 경고 H: 자동 반영 수단이 모두 없는 조합 */
  noAutoWarning: boolean
  /** 초안이 저장된 정책과 다르면 "다음 재스캔" 안내(저장 상태 기준)를 숨긴다 */
  draftChanged: boolean
  onChange: (patch: Partial<PolicyDraft>) => void
}

/** 주기 재스캔 일정 — 정책 종류 + 종류별 옵션 + 시간대 + 앱 시작 시 1회. */
export function RescanPolicySection({ folder, draft, storedUnknown, errors, defaultHours, noAutoWarning, draftChanged, onChange }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const k = "settings.folders.detection"
  const windowErrorId = `${id}-window-error`
  const Fields = draft.kind === POLICY_DEFAULT || draft.kind === POLICY_UNKNOWN ? null : TRIGGER_FIELDS[draft.kind]
  const rangeError = errors.hours ? t("settings.errorRange", INTERVAL_HOURS) : undefined
  const timeError = (e: "time" | undefined) => (e ? t(`${k}.errorTime`) : undefined)
  const savedNone = folder.nextRescanAt === null && storedKind(folder.rescanPolicy) === "manual"
  const hasOnStart = draft.kind !== POLICY_DEFAULT && draft.kind !== POLICY_UNKNOWN

  return (
    <section className="flex flex-col gap-3" aria-labelledby={`${id}-title`}>
      <h4 id={`${id}-title`} className="text-sm font-semibold">
        {t(`${k}.rescanSection`)}
      </h4>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${id}-kind`} className="!items-start">
          {t(`${k}.schedule`)}
        </Label>
        <Select value={draft.kind} onValueChange={(kind) => onChange({ kind: kind as DraftKind })}>
          <SelectTrigger id={`${id}-kind`} className="w-full min-w-0">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={POLICY_DEFAULT}>{t(`${k}.policyDefault`, { hours: defaultHours })}</SelectItem>
            {KNOWN_TRIGGER_KINDS.map((kind) => (
              <SelectItem key={kind} value={kind}>
                {t(KIND_LABEL[kind])}
              </SelectItem>
            ))}
            {storedUnknown && <SelectItem value={POLICY_UNKNOWN}>{t(`${k}.policyUnknown`)}</SelectItem>}
          </SelectContent>
        </Select>
      </div>
      {Fields && <Fields draft={draft} errors={{ hours: rangeError, at: timeError(errors.at) }} onChange={onChange} />}
      {windowVisible(draft) && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <Checkbox
              id={`${id}-window`}
              checked={draft.windowEnabled}
              onCheckedChange={(v) => onChange({ windowEnabled: v === true })}
            />
            <Label htmlFor={`${id}-window`}>{t(`${k}.windowEnabled`)}</Label>
          </div>
          {draft.windowEnabled && (
            <>
              <div className="flex flex-wrap items-start gap-3">
                <TimeField
                  label={t(`${k}.windowStart`)}
                  value={draft.windowStart}
                  error={timeError(errors.windowStart)}
                  externalErrorId={errors.window ? windowErrorId : undefined}
                  onChange={(windowStart) => onChange({ windowStart })}
                />
                <TimeField
                  label={t(`${k}.windowEnd`)}
                  value={draft.windowEnd}
                  error={timeError(errors.windowEnd)}
                  externalErrorId={errors.window ? windowErrorId : undefined}
                  onChange={(windowEnd) => onChange({ windowEnd })}
                />
              </div>
              {errors.window && (
                <p id={windowErrorId} role="alert" className="text-xs text-destructive break-keep">
                  {t(`${k}.errorWindowSame`)}
                </p>
              )}
              <p className="text-xs text-muted-foreground break-keep">{t(`${k}.windowHint`)}</p>
            </>
          )}
        </div>
      )}
      {hasOnStart && (
        <div className="flex items-center gap-2">
          <Checkbox id={`${id}-onstart`} checked={draft.onStart} onCheckedChange={(v) => onChange({ onStart: v === true })} />
          <Label htmlFor={`${id}-onstart`}>{t(`${k}.onStart`)}</Label>
        </div>
      )}
      {!draftChanged && folder.nextRescanAt !== null && (
        <p className="text-xs text-muted-foreground break-keep">{t(`${k}.nextAt`, { at: formatDateTime(folder.nextRescanAt) })}</p>
      )}
      {!draftChanged && savedNone && <p className="text-xs text-muted-foreground break-keep">{t(`${k}.nextNone`)}</p>}
      {noAutoWarning && (
        <p className="flex items-start gap-1.5 text-xs text-destructive break-keep">
          <TriangleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
          <span>{t(`${k}.noAutoWarning`)}</span>
        </p>
      )}
    </section>
  )
}
