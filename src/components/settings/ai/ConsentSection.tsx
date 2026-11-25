import { ShieldCheck, ShieldOff } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useRevokeConsent } from "@/hooks/useAi"
import { useConsentFlow } from "@/hooks/useConsent"
import { errorKey } from "@/lib/errors"
import { formatDate } from "@/lib/format"
import type { AiSettings } from "@/lib/ipc-contract-ai"

/** D1: 외부 AI 전송 동의 상태와 동의/철회. 철회하면 대기 중인 외부 분석이 취소되고 이미 받은 결과는 남는다. */
export function ConsentSection({ settings }: { settings: AiSettings }) {
  const { t } = useTranslation()
  const { askConsent } = useConsentFlow()
  const revoke = useRevokeConsent()
  const { given, at } = settings.consent
  const Icon = given ? ShieldCheck : ShieldOff
  return (
    <section aria-label={t("ai.consent.section")} className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <Icon aria-hidden="true" className="size-4 text-muted-foreground" />
        <span className="text-sm font-medium">
          {given ? t("ai.consent.given", { date: formatDate(at) }) : t("ai.consent.notGiven")}
        </span>
        {given ? (
          <Button size="sm" variant="outline" className="ml-auto" disabled={revoke.isPending} onClick={() => revoke.mutate()}>
            {t("ai.consent.revokeButton")}
          </Button>
        ) : (
          <Button size="sm" className="ml-auto" onClick={() => void askConsent()}>
            {t("ai.consent.giveButton")}
          </Button>
        )}
      </div>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.consent.sectionHint")}</p>
      {revoke.isError && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(revoke.error))}
        </p>
      )}
    </section>
  )
}
