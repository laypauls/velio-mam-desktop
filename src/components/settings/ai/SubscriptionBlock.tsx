import { useQueryClient } from "@tanstack/react-query"
import { useEffect, useId, useRef, useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { aiErrorText } from "@/components/ai/labels"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { AiCliLoginState, CliStatus, ProviderStatus } from "@/lib/ipc-contract-ai"

/** 로그인 진행 중 `ai_cli_login_status` 조회 간격 (ms) */
export const LOGIN_POLL_MS = 1500

interface Props {
  status: ProviderStatus
  /** 로그인 시작·대기·처리 중이면 true — 부모가 인증 방식 전환을 막는 데 쓴다 */
  onBusyChange?: (busy: boolean) => void
}

/**
 * Claude 구독 인증 블록 — 사용자가 설치한 Claude Code 의 브라우저 로그인에 연동한다.
 * 토큰은 보지 않는다. 로그인 진행 중에만(그리고 마운트된 동안만) 상태를 조회한다.
 */
export function SubscriptionBlock({ status, onBusyChange }: Props) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const { provider } = status
  const name = t(`ai.provider.name.${provider}`)
  const errorId = useId()
  const cli: CliStatus | null = status.cli
  const [login, setLogin] = useState<AiCliLoginState>({ state: "none" })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const waiting = login.state === "waiting"
  const loggedIn = cli?.account?.loggedIn === true

  const refresh = () => qc.invalidateQueries({ queryKey: qk.aiProviders })

  const busyNow = busy || waiting
  useEffect(() => {
    onBusyChange?.(busyNow)
    // onBusyChange 는 부모 렌더마다 바뀔 수 있어 값이 바뀔 때만 알린다
  }, [busyNow])
  useEffect(() => () => onBusyChange?.(false), [])

  /** 조회 결과 반영 — 마운트 조회와 폴링이 같은 규칙을 쓴다 (done/failed 는 Rust 가 한 번만 전달) */
  const applyStatus = (s: AiCliLoginState) => {
    if (s.state === "waiting") setLogin(s)
    else if (s.state === "done" || s.state === "failed") {
      setLogin({ state: "none" })
      if (s.state === "failed") setError(aiErrorText(t, s.errorCode))
      void refresh()
    }
  }

  // 재확인·공급자 갱신으로 이미 로그인된 것이 확인되면 남은 대기 세션을 정리해 폴링을 멈춘다
  useEffect(() => {
    if (!loggedIn || !waiting) return
    setLogin({ state: "none" })
    api.aiCliLoginCancel(provider).catch(() => undefined)
  }, [loggedIn, waiting, provider, api])

  // 다른 화면에 갔다 돌아와도 진행 중인 세션을 이어서 표시한다 (세션은 취소 전까지 유지됨)
  useEffect(() => {
    if (loggedIn) return
    let live = true
    api.aiCliLoginStatus(provider).then(
      (s) => live && applyStatus(s),
      () => undefined,
    )
    return () => {
      live = false
    }
    // 마운트 시 1회 — 이후 조회는 waiting 일 때의 폴링이 맡는다
  }, [])

  const inFlight = useRef(false)
  useEffect(() => {
    if (!waiting) return
    let live = true
    const id = setInterval(() => {
      if (inFlight.current) return
      inFlight.current = true
      api
        .aiCliLoginStatus(provider)
        .then((s) => {
          if (!live) return
          if (s.state === "none") setLogin(s)
          else applyStatus(s)
        })
        .catch((e: unknown) => {
          if (!live) return
          setLogin({ state: "none" })
          setError(t(errorKey(e)))
        })
        .finally(() => {
          inFlight.current = false
        })
    }, LOGIN_POLL_MS)
    return () => {
      live = false
      clearInterval(id)
    }
    // refresh/t 는 렌더마다 새로 만들어지므로 waiting·provider 가 바뀔 때만 타이머를 다시 건다
  }, [waiting, provider, api])

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await fn()
      await refresh()
    } catch (e) {
      setError(t(errorKey(e)))
    } finally {
      setBusy(false)
    }
  }
  const startLogin = async () => {
    setBusy(true)
    setError(null)
    try {
      const s = await api.aiCliLoginStart(provider)
      if (s.state === "failed") setError(aiErrorText(t, s.errorCode))
      else setLogin(s)
    } catch (e) {
      setError(t(errorKey(e)))
    } finally {
      setBusy(false)
    }
  }
  const cancel = async () => {
    setLogin({ state: "none" })
    await run(() => api.aiCliLoginCancel(provider))
  }

  const recheck = (
    <Button size="xs" variant="outline" disabled={busy} aria-label={t("ai.sub.recheckLabel", { name })} onClick={() => void run(() => api.detectAiCli(provider))}>
      {t("ai.sub.recheck")}
    </Button>
  )

  let body: ReactNode
  if (cli === null) {
    body = null
  } else if (!cli.found) {
    body = (
      <>
        <p className="text-xs break-keep">{t("ai.sub.notFound")}</p>
        <p className="text-xs break-keep text-muted-foreground">{t("ai.sub.installHint")}</p>
        <div className="flex gap-1">{recheck}</div>
      </>
    )
  } else if (!cli.supported) {
    body = (
      <>
        <p className="text-xs break-keep">{t("ai.sub.unsupported", { version: cli.version ?? "" })}</p>
        <div className="flex gap-1">{recheck}</div>
      </>
    )
  } else if (loggedIn) {
    const account = cli.account
    body = (
      <>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="rounded-sm">
            {t("ai.sub.installed", { version: cli.version ?? "" })}
          </Badge>
          <Badge variant="secondary" className="rounded-sm">
            {t("ai.sub.loggedIn")}
          </Badge>
          {account?.email && (
            <span className="text-xs break-all text-muted-foreground">
              {account.plan ? t("ai.sub.account", { email: account.email, plan: account.plan }) : account.email}
            </span>
          )}
        </div>
        <div className="flex gap-1">
          <Button size="xs" variant="outline" disabled={busy} aria-label={t("ai.sub.logoutLabel", { name })} onClick={() => void run(() => api.aiCliLogout(provider))}>
            {t("ai.sub.logout")}
          </Button>
          {recheck}
        </div>
      </>
    )
  } else if (waiting) {
    const url = login.state === "waiting" ? login.url : null
    body = (
      <>
        <p role="status" className="text-xs break-keep">
          {t("ai.sub.waiting")}
        </p>
        {url && (
          <div className="flex flex-col gap-1">
            <p className="text-xs break-keep text-muted-foreground">{t("ai.sub.urlHint")}</p>
            <div className="flex items-start gap-2">
              <code className="min-w-0 flex-1 rounded-sm bg-muted px-1.5 py-1 text-xs break-all">{url}</code>
              <Button size="xs" variant="outline" aria-label={t("ai.sub.openLabel", { name })} onClick={() => void run(() => api.openAiCliLoginUrl(provider))}>
                {t("ai.sub.open")}
              </Button>
            </div>
          </div>
        )}
        <p className="text-xs break-keep text-muted-foreground">{t("ai.sub.codeHint")}</p>
        <div className="flex gap-1">
          <Button size="xs" variant="ghost" disabled={busy} aria-label={t("ai.sub.cancelLabel", { name })} onClick={() => void cancel()}>
            {t("ai.sub.cancel")}
          </Button>
          {recheck}
        </div>
      </>
    )
  } else {
    body = (
      <>
        <p className="text-xs break-keep">{cli.account === null ? t("ai.sub.unknown") : t("ai.sub.notLoggedIn")}</p>
        <div className="flex gap-1">
          <Button size="xs" disabled={busy} aria-label={t("ai.sub.loginLabel", { name })} onClick={() => void startLogin()}>
            {t("ai.sub.login")}
          </Button>
          {recheck}
        </div>
      </>
    )
  }

  return (
    <div role="group" aria-label={t("ai.sub.title")} className="flex flex-col gap-1.5 rounded-md bg-muted/40 p-2">
      {body}
      {cli?.apiKeyEnv && (
        <p role="status" className="text-xs break-keep text-destructive">
          {t("ai.sub.apiKeyEnv")}
        </p>
      )}
      <p className="text-xs break-keep text-muted-foreground">{t("ai.sub.limits")}</p>
      {error && (
        <p id={errorId} role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}
