import { useId, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { DateInput } from "@/components/common/DateInput"
import { Input } from "@/components/ui/input"

/** 입력 텍스트 ↔ 저장 값(ms / bytes) 변환. 해석할 수 없는 텍스트는 null(= 제한 없음). */
export interface RangeCodec {
  type: "number" | "date"
  toText: (v: number | null) => string
  fromText: (text: string, end: boolean) => number | null
}

export function numberCodec(unit: number): RangeCodec {
  return {
    type: "number",
    toText: (v) => (v === null ? "" : String(v / unit)),
    fromText: (text) => {
      const n = Number(text)
      return text === "" || Number.isNaN(n) || n < 0 ? null : Math.round(n * unit)
    },
  }
}

export const dateCodec: RangeCodec = {
  type: "date",
  toText: (v) => {
    if (v === null) return ""
    const d = new Date(v)
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`
  },
  // `YYYY-MM-DD` → 로컬 자정(ms). `end` 면 해당 일의 마지막 ms
  fromText: (text, end) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
    if (!m) return null
    const start = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime()
    return end ? start + 86_400_000 - 1 : start
  },
}

interface Props {
  label: string
  minLabel: string
  maxLabel: string
  codec: RangeCodec
  min: number | null
  max: number | null
  /** 유효한 범위일 때만 호출된다 (min > max 면 호출하지 않고 인라인 오류를 보인다) */
  onApply: (min: number | null, max: number | null) => void
  /** 입력 폭 등 부가 클래스 */
  inputClassName?: string
}

/** 최소~최대 범위 입력 한 줄. 입력 중 텍스트를 보존하고, 역전된 범위는 적용하지 않는다. */
export function FilterRange({ label, minLabel, maxLabel, codec, min, max, onApply, inputClassName }: Props): ReactNode {
  const { t } = useTranslation()
  const errId = useId()
  const [minText, setMinText] = useState(codec.toText(min))
  const [maxText, setMaxText] = useState(codec.toText(max))
  const lo = codec.fromText(minText, false)
  const hi = codec.fromText(maxText, true)
  const invalid = lo !== null && hi !== null && lo > hi

  const change = (nextMin: string, nextMax: string) => {
    setMinText(nextMin)
    setMaxText(nextMax)
    const a = codec.fromText(nextMin, false)
    const b = codec.fromText(nextMax, true)
    if (a !== null && b !== null && a > b) return
    onApply(a, b)
  }

  const field = (ariaLabel: string, text: string, set: (v: string) => void) =>
    codec.type === "date" ? (
      <DateInput
        aria-label={ariaLabel}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errId : undefined}
        value={text}
        className={inputClassName}
        onValueChange={set}
      />
    ) : (
      <Input
        type="number"
        min={0}
        aria-label={ariaLabel}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errId : undefined}
        value={text}
        className={inputClassName}
        onChange={(e) => set(e.target.value)}
      />
    )

  return (
    <div className="flex flex-col !items-start gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <div className="flex items-center gap-2">
        {field(minLabel, minText, (v) => change(v, maxText))}
        <span aria-hidden="true">~</span>
        {field(maxLabel, maxText, (v) => change(minText, v))}
      </div>
      {invalid && (
        <p id={errId} role="alert" className="text-xs text-destructive break-keep">
          {t("filter.rangeInvalid")}
        </p>
      )}
    </div>
  )
}
