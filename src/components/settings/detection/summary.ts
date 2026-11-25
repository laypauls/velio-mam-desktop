import type { TFunction } from "i18next"
import { formatMonthDayTime, formatTime } from "@/lib/format"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { describePolicy, storedKind } from "@/lib/rescan-policy"
import { detectorName } from "./WatchModeSection"

/** 실제 동작 중인 변경 감지 방식 이름 — 감시하지 않으면 null. */
export function watchName(t: TFunction, folder: ManagedFolder): string | null {
  return folder.watchActive === null ? null : detectorName(t, folder.watchActive)
}

/** 폴백 사유 문구 — 폴백이 아니면 null. */
export function fallbackReason(t: TFunction, folder: ManagedFolder): string | null {
  return folder.watchFallbackKey === null ? null : t(`watch.reason.${folder.watchFallbackKey}`, { defaultValue: folder.watchFallbackKey })
}

/** 재스캔 정책 설명 (예: "6시간마다 (기본)"). */
export function rescanPolicyText(t: TFunction, folder: ManagedFolder, defaultHours: number): string {
  return describePolicy(t, folder.rescanPolicy, defaultHours)
}

/** 다음 예정 시각의 짧은 문구 — 오늘/내일은 시각만, 그 밖은 월·일 + 시각. 예정이 없으면 null. `now` 는 테스트용 주입. */
export function rescanNextText(t: TFunction, folder: ManagedFolder, now: number = Date.now()): string | null {
  const at = folder.nextRescanAt
  if (at === null) return null
  const dayStart = (ms: number) => new Date(new Date(ms).getFullYear(), new Date(ms).getMonth(), new Date(ms).getDate()).getTime()
  const diffDays = Math.round((dayStart(at) - dayStart(now)) / 86_400_000)
  const when =
    diffDays === 0
      ? t("settings.folders.detection.summary.nextToday", { time: formatTime(at) })
      : diffDays === 1
        ? t("settings.folders.detection.summary.nextTomorrow", { time: formatTime(at) })
        : formatMonthDayTime(at)
  return t("settings.folders.detection.summary.next", { at: when })
}

/** 재스캔만 하는 감지 + 수동 정책 — 변경이 자동으로 반영되지 않는다. */
export function isNoAutoReflect(folder: ManagedFolder): boolean {
  return folder.watchActive === "scan_only" && storedKind(folder.rescanPolicy) === "manual"
}
