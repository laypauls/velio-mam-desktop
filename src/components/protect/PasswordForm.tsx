import { useQueryClient } from "@tanstack/react-query"
import { useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { isCommandError } from "@/lib/errors"
import { PasswordInput } from "./PasswordInput"

export const MIN_PASSWORD_LENGTH = 8

interface Props {
  /** set: 최초 설정(새 비밀번호 + 확인), change: 현재 비밀번호 + 새 비밀번호 + 확인 */
  mode: "set" | "change"
  onDone?: () => void
}

type Errors = Partial<Record<"current" | "next" | "confirm" | "form", string>>

/** 보호 비밀번호 설정/변경 폼 — 필드별 인라인 검증 (최소 8자, 확인 일치). */
export function PasswordForm({ mode, onDone }: Props) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const [current, setCurrent] = useState("")
  const [next, setNext] = useState("")
  const [confirm, setConfirm] = useState("")
  const [errors, setErrors] = useState<Errors>({})
  const [busy, setBusy] = useState(false)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const found: Errors = {}
    if (mode === "change" && current === "") found.current = t("protect.prompt.errorEmpty")
    if (next.length < MIN_PASSWORD_LENGTH) found.next = t("protect.form.errorShort", { min: MIN_PASSWORD_LENGTH })
    else if (confirm !== next) found.confirm = t("protect.form.errorMismatch")
    setErrors(found)
    if (Object.keys(found).length > 0) return
    setBusy(true)
    try {
      if (mode === "set") await api.setPassword(next)
      else await api.changePassword(current, next)
      toast.success(t(mode === "set" ? "protect.form.setDone" : "protect.form.changeDone"))
      void qc.invalidateQueries({ queryKey: qk.protection })
      setCurrent("")
      setNext("")
      setConfirm("")
      onDone?.()
    } catch (err) {
      const code = isCommandError(err) ? err.code : "internal"
      const message = t(`errors.${code}`)
      setErrors(code === "password_incorrect" ? { current: message } : { form: message })
    } finally {
      setBusy(false)
    }
  }

  const clear = (key: keyof Errors) => setErrors((prev) => ({ ...prev, [key]: undefined, form: undefined }))

  return (
    <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-3">
      {mode === "change" && (
        <PasswordInput
          label={t("protect.form.current")}
          value={current}
          error={errors.current}
          onChange={(e) => {
            setCurrent(e.target.value)
            clear("current")
          }}
        />
      )}
      <PasswordInput
        label={t("protect.form.new")}
        value={next}
        error={errors.next}
        onChange={(e) => {
          setNext(e.target.value)
          clear("next")
        }}
      />
      <PasswordInput
        label={t("protect.form.confirm")}
        value={confirm}
        error={errors.confirm}
        onChange={(e) => {
          setConfirm(e.target.value)
          clear("confirm")
        }}
      />
      {errors.form && (
        <p role="alert" className="text-xs text-destructive">
          {errors.form}
        </p>
      )}
      <Button type="submit" size="sm" className="self-start" disabled={busy}>
        {t(mode === "set" ? "protect.form.set" : "protect.form.change")}
      </Button>
    </form>
  )
}
