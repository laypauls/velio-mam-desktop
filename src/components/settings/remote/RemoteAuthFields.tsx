import { useId } from "react"
import { useTranslation } from "react-i18next"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Textarea } from "@/components/ui/textarea"
import type { RemoteAuthKind } from "@/lib/ipc-contract"
import { RemoteTextField } from "./RemoteTextField"

interface Props {
  authKind: RemoteAuthKind
  secret: string
  /** 같은 인증 방식의 비밀이 이미 저장돼 있다 — 값은 절대 채우지 않고 "저장됨" 만 보여 준다 */
  saved: boolean
  error?: string
  disabled?: boolean
  onAuthKindChange: (kind: RemoteAuthKind) => void
  onSecretChange: (secret: string) => void
}

/** 인증 방식(비밀번호 / 개인 키) + 쓰기 전용 비밀 입력. 저장된 비밀은 읽어 오지 않는다. */
export function RemoteAuthFields({ authKind, secret, saved, error, disabled, onAuthKindChange, onSecretChange }: Props) {
  const { t } = useTranslation()
  const id = useId()
  const errorId = `${id}-error`
  const hintId = `${id}-hint`
  const hint = saved ? t("settings.folders.remote.secretSaved") : authKind === "key" ? t("settings.folders.remote.keyHint") : undefined
  return (
    <div className="flex flex-col gap-3">
      <fieldset className="flex flex-col gap-1.5">
        <legend className="mb-1 text-sm font-medium">{t("settings.folders.remote.auth")}</legend>
        <RadioGroup
          className="flex flex-wrap gap-x-5 gap-y-2"
          value={authKind}
          disabled={disabled}
          onValueChange={(v) => onAuthKindChange(v as RemoteAuthKind)}
        >
          {(["password", "key"] as const).map((k) => (
            <Label key={k} htmlFor={`${id}-${k}`} className="gap-2 font-normal">
              <RadioGroupItem id={`${id}-${k}`} value={k} />
              {t(`settings.folders.remote.authKind.${k}`)}
            </Label>
          ))}
        </RadioGroup>
      </fieldset>
      {authKind === "password" ? (
        <RemoteTextField
          label={t("settings.folders.remote.password")}
          type="password"
          autoComplete="new-password"
          value={secret}
          placeholder={saved ? t("settings.folders.remote.secretPlaceholder") : undefined}
          disabled={disabled}
          error={error}
          hint={hint}
          onValueChange={onSecretChange}
        />
      ) : (
        <div className="flex flex-col gap-1.5">
          <Label htmlFor={`${id}-secret`} className="!items-start">
            {t("settings.folders.remote.privateKey")}
          </Label>
          <Textarea
            id={`${id}-secret`}
            className="min-h-24 font-mono text-xs"
            spellCheck={false}
            autoComplete="off"
            value={secret}
            placeholder={saved ? t("settings.folders.remote.secretPlaceholder") : "-----BEGIN OPENSSH PRIVATE KEY-----"}
            disabled={disabled}
            aria-invalid={error ? true : undefined}
            aria-describedby={[error ? errorId : null, hint ? hintId : null].filter(Boolean).join(" ") || undefined}
            onChange={(e) => onSecretChange(e.target.value)}
          />
          {hint && (
            <p id={hintId} className="text-xs text-muted-foreground break-keep">
              {hint}
            </p>
          )}
          {error && (
            <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
