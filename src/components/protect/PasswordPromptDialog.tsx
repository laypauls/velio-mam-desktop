import { useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { useProtectStore } from "@/stores"
import { PasswordInput } from "./PasswordInput"

function PromptForm({ resolve }: { resolve: (ok: boolean) => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const setPrompt = useProtectStore((s) => s.setPasswordPrompt)
  const [password, setPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const finish = (ok: boolean) => {
    setPrompt(null)
    resolve(ok)
  }

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (password === "") {
      setError(t("protect.prompt.errorEmpty"))
      return
    }
    setBusy(true)
    try {
      await api.verifyPassword(password)
      finish(true)
    } catch (err) {
      setError(t(errorKey(err)))
      setPassword("")
      setBusy(false)
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle>{t("protect.prompt.title")}</DialogTitle>
        <DialogDescription>{t("protect.prompt.description")}</DialogDescription>
      </DialogHeader>
      <PasswordInput
        label={t("protect.password")}
        value={password}
        error={error}
        autoFocus
        onChange={(e) => {
          setPassword(e.target.value)
          setError(null)
        }}
      />
      <DialogFooter>
        <Button type="button" variant="outline" onClick={() => finish(false)}>
          {t("common.cancel")}
        </Button>
        <Button type="submit" disabled={busy}>
          {t("protect.prompt.confirm")}
        </Button>
      </DialogFooter>
    </form>
  )
}

/** 비밀번호 확인 Dialog — `useProtectedAction` 이 요청하면 열리고, 인증 성공/취소 결과를 돌려준다. */
export function PasswordPromptDialog() {
  const prompt = useProtectStore((s) => s.passwordPrompt)
  const setPrompt = useProtectStore((s) => s.setPasswordPrompt)
  return (
    <Dialog
      open={prompt !== null}
      onOpenChange={(open) => {
        if (open || prompt === null) return
        setPrompt(null)
        prompt.resolve(false)
      }}
    >
      <DialogContent className="sm:max-w-md">{prompt && <PromptForm resolve={prompt.resolve} />}</DialogContent>
    </Dialog>
  )
}
