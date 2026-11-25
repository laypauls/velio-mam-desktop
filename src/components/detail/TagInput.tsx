import { useId, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Input } from "@/components/ui/input"
import { useTags } from "@/hooks/useLibrary"
import { errorKey } from "@/lib/errors"

const MAX_SUGGESTIONS = 6

interface Props {
  /** 이미 붙어 있는(또는 담아 둔) 태그 이름 — 추천에서 제외한다 */
  existing: readonly string[]
  /** Enter / 추천 클릭 시 호출. reject 하면 인라인 오류로 보여준다 */
  onAdd: (name: string) => void | Promise<unknown>
  label: string
}

/** 태그 추가 입력 — Enter 로 추가, `listTags` 기반 자동완성, 빈 값은 인라인 오류(aria-invalid). */
export function TagInput({ existing, onAdd, label }: Props) {
  const { t } = useTranslation()
  const tags = useTags()
  const errorId = useId()
  const [value, setValue] = useState("")
  const [error, setError] = useState<string | null>(null)

  const taken = new Set(existing.map((n) => n.normalize("NFC").toLowerCase()))
  const needle = value.trim().toLowerCase()
  const suggestions =
    needle === ""
      ? []
      : (tags.data ?? []).filter((x) => x.name.toLowerCase().includes(needle) && !taken.has(x.name.normalize("NFC").toLowerCase())).slice(0, MAX_SUGGESTIONS)

  const submit = async (raw: string) => {
    const name = raw.trim()
    if (name === "") {
      setError(t("tags.errorEmpty"))
      return
    }
    try {
      await onAdd(name)
      setValue("")
      setError(null)
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing) return
    e.preventDefault()
    void submit(value)
  }

  return (
    <div className="flex flex-col gap-1">
      <Input
        value={value}
        placeholder={t("tags.placeholder")}
        aria-label={label}
        aria-invalid={error !== null}
        aria-describedby={error !== null ? errorId : undefined}
        onChange={(e) => {
          setValue(e.target.value)
          setError(null)
        }}
        onKeyDown={onKeyDown}
      />
      {error !== null && (
        <p id={errorId} role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
      {suggestions.length > 0 && (
        <ul aria-label={t("tags.suggestions")} className="flex flex-wrap gap-1">
          {suggestions.map((s) => (
            <li key={s.tagId}>
              <button
                type="button"
                onClick={() => void submit(s.name)}
                className="rounded-sm border border-border bg-background px-1.5 py-0.5 text-xs hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                {s.name}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
