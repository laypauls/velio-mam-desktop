import { useEffect, useId, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Textarea } from "@/components/ui/textarea"
import { useSetMemo } from "@/hooks/useUserData"
import { errorKey } from "@/lib/errors"

/** 메모 — 포커스를 잃거나 Ctrl+Enter 로 저장하고, 저장 상태를 표시한다. */
export function MemoEditor({ assetId, memo }: { assetId: number; memo: string }) {
  const { t } = useTranslation()
  const setMemo = useSetMemo()
  const errorId = useId()
  const [value, setValue] = useState(memo)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // 다른 자산을 고르면 상태 표시를 지우고, 서버 값이 바뀌면 편집 값을 맞춘다
  useEffect(() => {
    setSaved(false)
    setError(null)
  }, [assetId])
  useEffect(() => {
    setValue(memo)
  }, [assetId, memo])

  const save = () => {
    if (value === memo || setMemo.isPending) return
    setError(null)
    setMemo.mutate(
      { assetId, memo: value },
      {
        onSuccess: () => setSaved(true),
        onError: (e) => setError(t(errorKey(e))),
      }
    )
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault()
      save()
    }
  }

  return (
    <div className="flex flex-col gap-1">
      <Textarea
        value={value}
        rows={3}
        aria-label={t("detail.memo")}
        aria-invalid={error !== null}
        aria-describedby={error !== null ? errorId : undefined}
        placeholder={t("memo.placeholder")}
        onChange={(e) => {
          setValue(e.target.value)
          setSaved(false)
        }}
        onBlur={save}
        onKeyDown={onKeyDown}
      />
      <div className="min-h-4 text-xs text-muted-foreground">
        {error !== null ? (
          <p id={errorId} role="alert" className="text-destructive">
            {error}
          </p>
        ) : setMemo.isPending ? (
          <span role="status">{t("memo.saving")}</span>
        ) : saved ? (
          <span role="status">{t("memo.saved")}</span>
        ) : (
          <span className="break-keep">{t("memo.hint")}</span>
        )}
      </div>
    </div>
  )
}
