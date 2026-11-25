import type { ComponentType } from "react"
import type { KnownTriggerKind } from "@/lib/ipc-contract"
import type { PolicyDraft } from "@/lib/rescan-policy"
import { IntervalFields } from "./IntervalFields"
import { ScheduleFields } from "./ScheduleFields"

export interface TriggerFieldProps {
  draft: PolicyDraft
  /** 이미 번역된 인라인 오류 문구 */
  errors: { hours?: string; at?: string }
  onChange: (patch: Partial<PolicyDraft>) => void
}

/**
 * 트리거 종류 → 옵션 입력 컴포넌트. 새 트리거를 추가하면 여기에 한 줄을 더한다
 * (`KnownTriggerKind` 가 늘면 이 표가 컴파일되지 않는다). 옵션이 없는 종류는 null.
 */
export const TRIGGER_FIELDS: { [K in KnownTriggerKind]: ComponentType<TriggerFieldProps> | null } = {
  interval: IntervalFields,
  schedule: ScheduleFields,
  manual: null,
}

