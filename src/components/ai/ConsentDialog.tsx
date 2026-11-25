import { useQueryClient } from "@tanstack/react-query"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { qk } from "@/hooks/keys"
import { useAiSettings } from "@/hooks/useAi"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { AI_DEFAULTS } from "@/lib/ipc-contract-ai"
import { useAiStore } from "@/stores"

function ConsentForm({ resolve }: { resolve: (agreed: boolean) => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const setPrompt = useAiStore((s) => s.setConsentPrompt)
  const settings = useAiSettings()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const transfer = settings.data?.transfer ?? AI_DEFAULTS.transfer

  const finish = (agreed: boolean) => {
    setPrompt(null)
    resolve(agreed)
  }
  const agree = async () => {
    setBusy(true)
    setError(null)
    try {
      qc.setQueryData(qk.aiSettings, await api.giveAiConsent())
      finish(true)
    } catch (e) {
      setError(t(errorKey(e)))
      setBusy(false)
    }
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("ai.consent.title")}</DialogTitle>
        <DialogDescription>{t("ai.consent.description")}</DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-3 text-sm">
        <section className="flex flex-col gap-1">
          <h3 className="font-medium">{t("ai.consent.sendTitle")}</h3>
          <ul className="list-disc pl-5 text-muted-foreground break-keep">
            <li>{t("ai.consent.sendImage", { edge: transfer.imageMaxEdge })}</li>
            <li>{t("ai.consent.sendFrames", { count: transfer.videoFrames })}</li>
            <li>{t("ai.consent.sendMeta")}</li>
          </ul>
        </section>
        <section className="flex flex-col gap-1">
          <h3 className="font-medium">{t("ai.consent.neverTitle")}</h3>
          <ul className="list-disc pl-5 text-muted-foreground break-keep">
            <li>{t("ai.consent.neverOriginal")}</li>
            <li>{t("ai.consent.neverAudio")}</li>
          </ul>
        </section>
        <p className="text-muted-foreground break-keep">{t("ai.consent.revoke")}</p>
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={() => finish(false)}>
          {t("ai.consent.decline")}
        </Button>
        <Button type="button" disabled={busy} onClick={() => void agree()}>
          {t("ai.consent.agree")}
        </Button>
      </DialogFooter>
    </>
  )
}

/** D1: 외부 AI 전송 동의 Dialog — `useConsentFlow` 가 요청하면 열리고 동의 여부를 돌려준다. */
export function ConsentDialog() {
  const prompt = useAiStore((s) => s.consentPrompt)
  const setPrompt = useAiStore((s) => s.setConsentPrompt)
  return (
    <Dialog
      open={prompt !== null}
      onOpenChange={(open) => {
        if (open || prompt === null) return
        setPrompt(null)
        prompt.resolve(false)
      }}
    >
      <DialogContent className="sm:max-w-md">{prompt && <ConsentForm resolve={prompt.resolve} />}</DialogContent>
    </Dialog>
  )
}
