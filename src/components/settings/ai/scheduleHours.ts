/** 24시간 막대 위의 한 구간 — 시(0~24) 단위, from 포함 · to 제외 */
export interface HourSegment {
  from: number
  to: number
}

export type ScheduleKind = "allDay" | "window" | "wrap"

/**
 * 예약 시간대(시작·종료 시, 코어와 같은 규칙: 시작 포함 · 종료 제외)를 막대에 칠할 구간으로 바꾼다.
 * 시작 = 종료 → 하루 종일(0~24), 시작 < 종료 → 한 구간, 시작 > 종료 → 자정을 넘겨 두 구간(시작~24, 0~종료).
 * 종료 0 은 자정이라 앞 구간 하나만 남는다. 범위를 벗어난 값은 0~24 로 맞춘다.
 */
export function scheduleSegments(startHour: number, endHour: number): { kind: ScheduleKind; segments: HourSegment[] } {
  const clamp = (h: number) => Math.min(24, Math.max(0, Math.trunc(h)))
  const start = clamp(startHour)
  const end = clamp(endHour)
  if (start === end || (start === 0 && end === 24)) return { kind: "allDay", segments: [{ from: 0, to: 24 }] }
  if (start < end) return { kind: "window", segments: [{ from: start, to: end }] }
  const segments: HourSegment[] = [{ from: start, to: 24 }]
  if (end > 0) segments.push({ from: 0, to: end })
  return { kind: end > 0 ? "wrap" : "window", segments }
}

/** 시간별(0~23시) 활성 여부 24칸 — 막대는 이 칸들을 그대로 칠한다 */
export function activeHours(startHour: number, endHour: number): boolean[] {
  const { segments } = scheduleSegments(startHour, endHour)
  return Array.from({ length: 24 }, (_, h) => segments.some((s) => h >= s.from && h < s.to))
}
