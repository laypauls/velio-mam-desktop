import { useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import type { SceneSensitivity } from "@/lib/ipc-contract"
import type { RedetectChoice } from "./useSceneController"

export const SCENE_SENSITIVITY_OPTIONS: readonly SceneSensitivity[] = ["low", "medium", "high"]

/** "자동 장면전환점 다시 추출…" — 이 영상에만 쓸 민감도와 "지운 점 되살리기" 선택. 사용자가 추가한 컷은 항상 유지된다 */
export function SceneRedetectDialog({
  open,
  defaultSensitivity,
  onOpenChange,
  onConfirm,
}: {
  open: boolean
  defaultSensitivity: SceneSensitivity
  onOpenChange: (open: boolean) => void
  onConfirm: (choice: RedetectChoice) => void
}) {
  const { t } = useTranslation()
  const [sensitivity, setSensitivity] = useState<SceneSensitivity | null>(null)
  const [restoreSuppressed, setRestore] = useState(false)
  const selectId = useId()
  const restoreId = useId()
  const shown = sensitivity ?? defaultSensitivity
  const close = (next: boolean) => {
    if (!next) {
      setSensitivity(null)
      setRestore(false)
    }
    onOpenChange(next)
  }
  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("scene.redetect.title")}</DialogTitle>
          <DialogDescription className="break-keep">{t("scene.redetect.description")}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-4">
            <Label htmlFor={selectId} className="break-keep">
              {t("scene.sensitivity")}
            </Label>
            <Select value={shown} onValueChange={(v) => setSensitivity(v as SceneSensitivity)}>
              <SelectTrigger id={selectId} className="w-40">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {SCENE_SENSITIVITY_OPTIONS.map((s) => (
                  <SelectItem key={s} value={s}>
                    {t(`scene.sensitivity_${s}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-start gap-2">
            <Checkbox id={restoreId} checked={restoreSuppressed} onCheckedChange={(v) => setRestore(v === true)} />
            <Label htmlFor={restoreId} className="!items-start flex-col gap-0.5 break-keep">
              <span>{t("scene.redetect.restore")}</span>
              <span className="text-xs font-normal text-muted-foreground">{t("scene.redetect.restoreHint")}</span>
            </Label>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => close(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={() => {
              onConfirm({ sensitivity: shown, restoreSuppressed })
              close(false)
            }}
          >
            {t("scene.redetect.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
