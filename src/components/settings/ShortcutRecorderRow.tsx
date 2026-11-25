import { useId, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useUpdateSettings } from "@/hooks/useSettings"
import { presetConflict } from "@/components/viewer/playRangeKeys"
import type { PlayRangeKeys } from "@/lib/ipc-contract"
import { formatShortcut, isReservedShortcut, isValidShortcut, shortcutFromEvent } from "@/lib/shortcut"

/** 단축키 기록 행이 편집하는 설정 키 */
export type ShortcutField = "hideShortcut" | "closeViewerShortcut"

interface Props {
  field: ShortcutField
  label: string
  hint: string
  value: string | null
  defaultValue: string
  /** 같은 소제목의 다른 단축키 행 값 — 같은 조합은 거부한다(양쪽 행에서 검사) */
  otherValue: string | null
  rangeKeys: PlayRangeKeys
}

/** 한글 등 비라틴 입력 상태의 문자 키("ㅂ")는 물리 키(KeyQ)의 라틴 문자로 기록한다 — 충돌 검사와 표기를 라틴 키 기준으로 맞춘다 */
function combo(e: KeyboardEvent<HTMLButtonElement>): string | null {
  const c = shortcutFromEvent(e.nativeEvent)
  const m = /^Key([A-Z])$/.exec(e.nativeEvent.code)
  if (c === null || m === null) return c
  const key = c.slice(c.lastIndexOf("+") + 1)
  return key.length === 1 && !/^[A-Za-z]$/.test(key) ? `${c.slice(0, c.length - key.length)}${m[1]}` : c
}

/**
 * 설정의 단축키 기록 행 — 키 기록 칸 + 기본값 + 사용 안 함. 숨기기·패널 닫기 단축키가 함께 쓴다.
 * 이미 쓰이는 키(예약 키·선택한 재생 구간 프리셋의 키·다른 단축키 행의 값)는 저장을 거부하고 인라인 에러를 보인다.
 */
export function ShortcutRecorderRow({ field, label, hint, value, defaultValue, otherValue, rangeKeys }: Props) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const [recording, setRecording] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const hintId = useId()
  const errorId = useId()

  const save = (next: string | null) => {
    setError(null)
    setRecording(false)
    update.mutate({ [field]: next })
  }

  /** 검증을 통과하면 저장하고, 아니면 인라인 에러를 보인다 */
  const tryApply = (next: string) => {
    if (!isValidShortcut(next)) {
      setRecording(false)
      return setError(t("settings.view.hideShortcutInvalid", { key: formatShortcut(next) }))
    }
    if (isReservedShortcut(next) || next === otherValue || presetConflict(rangeKeys, next) !== null) {
      setRecording(false)
      return setError(t("settings.view.hideShortcutReserved", { key: formatShortcut(next) }))
    }
    save(next)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (!recording || e.repeat) return
    // Tab 은 기록을 취소하고 포커스 이동은 그대로 둔다
    if (e.key === "Tab") return setRecording(false)
    e.preventDefault()
    const next = combo(e)
    if (next === null) return // 수정자만 누른 상태 — 다음 키를 기다린다
    tryApply(next)
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-sm font-medium">{label}</span>
          <span id={hintId} className="text-xs text-muted-foreground break-keep">
            {hint}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            aria-label={label}
            aria-invalid={error !== null || undefined}
            aria-describedby={error !== null ? `${hintId} ${errorId}` : hintId}
            data-shortcut-capture={recording || undefined}
            className="w-36 justify-center"
            onClick={() => {
              setError(null)
              setRecording(true)
            }}
            onKeyDown={onKeyDown}
            onBlur={() => setRecording(false)}
          >
            {recording ? t("settings.view.hideShortcutRecording") : value === null ? t("settings.view.hideShortcutNone") : formatShortcut(value)}
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={value === defaultValue} onClick={() => tryApply(defaultValue)}>
            {t("settings.view.hideShortcutDefault")}
          </Button>
          <Button type="button" variant="ghost" size="sm" disabled={value === null} onClick={() => save(null)}>
            {t("settings.view.hideShortcutDisable")}
          </Button>
        </div>
      </div>
      {error !== null && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </div>
  )
}
