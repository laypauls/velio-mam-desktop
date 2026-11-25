import { useEffect, useId, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { TITLE_MAX_LENGTH } from "@/components/detail/TitleEditor"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { useSetAssetTitle } from "@/hooks/useUserData"
import { effectiveTitle } from "@/lib/displayName"
import { errorKey } from "@/lib/errors"
import type { AssetSummary } from "@/lib/ipc-contract"

/** 우클릭 "제목 변경…" — 제목 하나를 입력받아 저장한다. 비우면 제목 해제(파일명 표시). */
export function TitleDialog({ asset, onClose }: { asset: AssetSummary | null; onClose: () => void }) {
  const { t } = useTranslation()
  const setTitle = useSetAssetTitle()
  const errorId = useId()
  const [value, setValue] = useState("")
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setValue(asset ? effectiveTitle(asset) : "")
    setError(null)
  }, [asset])

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (asset === null || setTitle.isPending) return
    const trimmed = value.trim()
    // 기본값(파일명)이나 저장된 제목과 같으면 저장하지 않는다. 비우면 제목 해제
    if (trimmed === effectiveTitle(asset) || (trimmed === "" && asset.title === null)) return onClose()
    setError(null)
    setTitle.mutate(
      { assetId: asset.assetId, title: trimmed === "" ? null : trimmed },
      { onSuccess: onClose, onError: (err) => setError(t(errorKey(err))) }
    )
  }

  return (
    <Dialog open={asset !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>{t("titleEdit.dialogTitle")}</DialogTitle>
            <DialogDescription>{t("titleEdit.dialogDescription")}</DialogDescription>
          </DialogHeader>
          <Input
            autoFocus
            value={value}
            maxLength={TITLE_MAX_LENGTH}
            aria-label={t("titleEdit.label")}
            aria-invalid={error !== null}
            aria-describedby={error !== null ? errorId : undefined}
            placeholder={asset?.fileName}
            onChange={(e) => setValue(e.target.value)}
          />
          {error !== null && (
            <p id={errorId} role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={setTitle.isPending}>
              {t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
