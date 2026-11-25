import { useQueryClient } from "@tanstack/react-query"
import { useId, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { aiErrorText } from "@/components/ai/labels"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { AiProvider, AuthMode, ProviderStatus } from "@/lib/ipc-contract-ai"
import { SubscriptionBlock } from "./SubscriptionBlock"

interface Props {
  status: ProviderStatus
  /** "이 PC 에 저장(Windows 보호)" — 해제하면 이번 실행 동안만 메모리에 둔다 */
  persist: boolean
}

function CheckResult({ status }: { status: ProviderStatus }) {
  const { t } = useTranslation()
  const check = status.lastCheck
  if (!check) return null
  const text = check.ok
    ? t("ai.provider.checkOk")
    : check.errorCode === "offline"
      ? t("ai.offlineNotice")
      : aiErrorText(t, check.errorCode ?? "unknown")
  return (
    <p role="status" className={check.ok ? "text-xs text-muted-foreground" : "text-xs break-keep text-destructive"}>
      {text}
    </p>
  )
}

/**
 * D2: 공급자 한 줄 — 키 보관 상태 배지, 쓰기 전용 키 입력, 삭제, 연결 확인.
 * 키가 mutation 캐시(variables)에 남지 않도록 `api` 를 직접 호출하고, 저장이 성공하면 입력란을 비운다.
 */
export function ProviderRow({ status, persist }: Props) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const { provider } = status
  const subscription = status.authMode === "subscription"
  const name = t(`ai.provider.name.${provider}`)
  const errorId = useId()
  const [key, setKey] = useState("")
  const [busy, setBusy] = useState(false)
  const [loginBusy, setLoginBusy] = useState(false) // 구독 로그인 진행 중 — 인증 방식 전환 금지
  const [error, setError] = useState<string | null>(null)

  const refresh = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: qk.aiProviders }),
      qc.invalidateQueries({ queryKey: qk.aiSettings }),
    ])
  const run = async (fn: (p: AiProvider) => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await fn(provider)
      await refresh()
      return true
    } catch (e) {
      setError(t(errorKey(e)))
      return false
    } finally {
      setBusy(false)
    }
  }

  const save = async (e: FormEvent) => {
    e.preventDefault()
    if (key.trim() === "") {
      setError(t("ai.provider.keyEmpty"))
      return
    }
    if (await run((p) => api.setApiKey(p, key, persist))) setKey("")
  }

  return (
    <li className="flex flex-col gap-1.5 rounded-lg border border-border p-3">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{name}</span>
        {!subscription && (
          <Badge variant={status.keyStorage === "none" ? "outline" : "secondary"} className="rounded-sm">
            {t("ai.provider.keyState", { state: t(`ai.key.${status.keyStorage}`) })}
          </Badge>
        )}
        <div className="ml-auto flex gap-1">
          <Button size="xs" variant="outline" disabled={busy} aria-label={t("ai.provider.check", { name })} onClick={() => void run((p) => api.checkProvider(p))}>
            {t("ai.provider.checkShort")}
          </Button>
          {!subscription && (
            <Button size="xs" variant="ghost" disabled={busy || status.keyStorage === "none"} aria-label={t("ai.provider.clear", { name })} onClick={() => void run((p) => api.clearApiKey(p))}>
              {t("ai.provider.clearShort")}
            </Button>
          )}
        </div>
      </div>
      {status.subscriptionSupported && (
        <RadioGroup
          value={status.authMode}
          aria-label={t("ai.provider.modeInput", { name })}
          className="flex w-full flex-wrap items-center gap-x-4 gap-y-1"
          disabled={busy || loginBusy}
          onValueChange={(v) => void run((p) => api.setProviderAuthMode(p, v as AuthMode))}
        >
          <span className="text-xs text-muted-foreground">{t("ai.provider.authMode")}</span>
          {(["apiKey", "subscription"] as const).map((m) => (
            <Label key={m} className="!items-center gap-1.5 text-xs font-normal">
              <RadioGroupItem value={m} />
              {t(`ai.provider.mode.${m}`)}
            </Label>
          ))}
        </RadioGroup>
      )}
      {subscription ? (
        <SubscriptionBlock status={status} onBusyChange={setLoginBusy} />
      ) : (
        <form className="flex gap-2" onSubmit={(e) => void save(e)}>
          <Input
            type="password"
            autoComplete="off"
            value={key}
            placeholder={t("ai.provider.keyPlaceholder")}
            aria-label={t("ai.provider.keyInput", { name })}
            aria-invalid={error !== null ? true : undefined}
            aria-describedby={error !== null ? errorId : undefined}
            onChange={(e) => {
              setKey(e.target.value)
              setError(null)
            }}
          />
          <Button type="submit" size="sm" disabled={busy} aria-label={t("ai.provider.save", { name })}>
            {t("common.save")}
          </Button>
        </form>
      )}
      {error && (
        <p id={errorId} role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <CheckResult status={status} />
    </li>
  )
}
