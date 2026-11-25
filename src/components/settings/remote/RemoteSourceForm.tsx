import { Loader2 } from "lucide-react"
import { useId, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { DialogFooter } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import type { RemoteSource, RemoteSourceKind } from "@/lib/ipc-contract"
import { PathMapRows } from "./PathMapRows"
import { RemoteAuthFields } from "./RemoteAuthFields"
import { RemoteTextField } from "./RemoteTextField"
import { TestResultLine } from "./TestResultLine"
import type { RemoteErrors } from "./remote-form"
import { useRemoteSourceForm } from "./useRemoteSourceForm"

const KINDS: readonly RemoteSourceKind[] = ["ddp_audit", "ses_queue"]

interface Props {
  /** 수정할 접속 (null = 새 접속) */
  stored: RemoteSource | null
  /** 새 접속의 처음 종류 */
  initialKind: RemoteSourceKind
  onCancel: () => void
  onSaved: (saved: RemoteSource) => void
}

/** 원격 스토리지 접속 만들기·수정 폼. 종류는 만든 뒤 바꿀 수 없고, 비밀은 쓰기 전용이다. */
export function RemoteSourceForm({ stored, initialKind, onCancel, onSaved }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const f = useRemoteSourceForm(stored, initialKind, onSaved)
  const { draft, errors } = f
  const err = (field: keyof RemoteErrors) => (errors[field] === undefined ? undefined : t(`settings.folders.remote.error.${errors[field]}`))
  const queue = draft.kind === "ses_queue"
  const secretSaved = stored !== null && stored.hasSecret && stored.authKind === draft.authKind

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    void f.submit()
  }

  return (
    <form onSubmit={onSubmit} className="flex min-h-0 flex-col gap-4" aria-busy={f.busy}>
      {/* 본문만 스크롤 — relative 로 내부 absolute/sr-only 자손이 바깥 조상 기준으로 배치되는 것을 막는다 */}
      <div className="relative -mx-1 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-1">
        <fieldset className="flex flex-col gap-1.5">
          <legend className="mb-1 text-sm font-medium">{t("settings.folders.remote.kind")}</legend>
          <RadioGroup
            className="flex flex-wrap gap-x-5 gap-y-2"
            value={draft.kind}
            disabled={stored !== null || f.busy}
            onValueChange={(v) => f.set({ kind: v as RemoteSourceKind, pathMap: [{ from: v === "ses_queue" ? "/data/incoming" : "/", to: "" }] })}
          >
            {KINDS.map((k) => (
              <Label key={k} htmlFor={`${id}-kind-${k}`} className="gap-2 font-normal">
                <RadioGroupItem id={`${id}-kind-${k}`} value={k} />
                {t(`settings.folders.remote.kinds.${k}`)}
              </Label>
            ))}
          </RadioGroup>
          {stored !== null && <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.remote.kindFixed")}</p>}
        </fieldset>

        <RemoteTextField label={t("settings.folders.remote.name")} value={draft.name} disabled={f.busy} error={err("name")} onValueChange={(name) => f.set({ name })} />
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-[minmax(0,1fr)_6rem]">
          <RemoteTextField label={t("settings.folders.remote.host")} value={draft.host} disabled={f.busy} error={err("host")} onValueChange={(host) => f.set({ host })} />
          <RemoteTextField
            label={t("settings.folders.remote.port")}
            type="number"
            inputMode="numeric"
            value={draft.port}
            disabled={f.busy}
            error={err("port")}
            onValueChange={(port) => f.set({ port })}
          />
        </div>
        <RemoteTextField label={t("settings.folders.remote.user")} autoComplete="off" value={draft.user} disabled={f.busy} error={err("user")} onValueChange={(user) => f.set({ user })} />

        <RemoteAuthFields
          authKind={draft.authKind}
          secret={draft.secret}
          saved={secretSaved}
          error={err("secret")}
          disabled={f.busy}
          onAuthKindChange={(authKind) => f.set({ authKind, secret: "" })}
          onSecretChange={(secret) => f.set({ secret })}
        />

        {queue ? (
          <RemoteTextField
            label={t("settings.folders.remote.dbPath")}
            inputClassName="font-mono"
            placeholder="/srv/ses/events.db"
            value={draft.dbPath}
            disabled={f.busy}
            error={err("dbPath")}
            hint={t("settings.folders.remote.dbPathHint")}
            onValueChange={(dbPath) => f.set({ dbPath })}
          />
        ) : (
          <RemoteTextField
            label={t("settings.folders.remote.logDir")}
            inputClassName="font-mono"
            placeholder="/tmp/avfs-dbase/audit"
            value={draft.logDir}
            disabled={f.busy}
            error={err("logDir")}
            hint={t("settings.folders.remote.logDirHint")}
            onValueChange={(logDir) => f.set({ logDir })}
          />
        )}
        <RemoteTextField
          label={t("settings.folders.remote.pollSec")}
          type="number"
          inputMode="numeric"
          inputClassName="w-40"
          value={draft.pollSec}
          disabled={f.busy}
          error={err("pollSec")}
          onValueChange={(pollSec) => f.set({ pollSec })}
        />

        <PathMapRows rows={draft.pathMap} disabled={f.busy} error={err("pathMap")} onChange={f.setPathMap} />

        <div className="flex flex-col gap-1 text-xs">
          <span className="font-medium">{t("settings.folders.remote.hostKey")}</span>
          {draft.hostKey === null ? (
            <span className="text-muted-foreground break-keep">{t("settings.folders.remote.hostKeyNone")}</span>
          ) : (
            <code className="break-all font-mono">{draft.hostKey}</code>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <div>
            <Button type="button" variant="outline" size="sm" disabled={f.busy} aria-busy={f.testing} onClick={() => void f.runTest()}>
              {f.testing && <Loader2 aria-hidden="true" className="animate-spin" />}
              {t("settings.folders.remote.test")}
            </Button>
          </div>
          {f.result && <TestResultLine result={f.result} kind={draft.kind} hostKey={draft.hostKey} disabled={f.busy} onConfirm={f.confirmHostKey} />}
        </div>

        {f.failure !== null && (
          <p role="alert" className="text-xs text-destructive break-keep">
            {t(f.failure)}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" disabled={f.saving} onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button type="submit" disabled={f.busy} aria-busy={f.saving}>
          {t("common.save")}
        </Button>
      </DialogFooter>
    </form>
  )
}
