import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { useDismissVolumes, usePendingVolumes } from "@/hooks/useNewVolumes"
import { NewVolumeCard } from "./NewVolumeCard"

/**
 * 새 디스크 연결 확인 — 셸에 한 번 마운트한다. 대기 목록이 비어 있지 않은 동안 열려 있고, 카드는 백엔드 이벤트로 사라진다.
 * 카드별 "나중에"와 푸터 "닫기"(= 남은 전체 나중에)만 "이 기기에서 선택한 사항 기억하기" 체크를 따른다.
 * Esc·X 는 실수로 누를 수 있으므로 체크와 무관하게 이번 연결 동안만(remember=false) 넘긴다.
 */
export function NewVolumeDialog() {
  const { t } = useTranslation()
  const volumes = usePendingVolumes()
  const dismiss = useDismissVolumes()
  const [remember, setRemember] = useState(true)
  const closeAll = (rememberChoice: boolean) => {
    if (volumes.length > 0 && !dismiss.isPending) dismiss.mutate({ volumeKeys: volumes.map((v) => v.volumeKey), remember: rememberChoice })
  }

  return (
    <Dialog open={volumes.length > 0} onOpenChange={(o) => !o && closeAll(false)}>
      <DialogContent className="sm:max-w-xl" onInteractOutside={(e) => e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>{t("volumes.new.title")}</DialogTitle>
          <DialogDescription className="break-keep">{t("volumes.new.description")}</DialogDescription>
        </DialogHeader>
        <div className="relative flex max-h-[60vh] flex-col gap-3 overflow-y-auto">
          {volumes.map((v) => (
            <NewVolumeCard
              key={v.volumeKey}
              volume={v}
              busy={dismiss.isPending}
              onLater={() => dismiss.mutate({ volumeKeys: [v.volumeKey], remember })}
            />
          ))}
        </div>
        <p className="text-xs text-muted-foreground break-keep">{t("volumes.new.systemExcluded")}</p>
        <div className="flex items-center gap-2">
          <Checkbox id="new-volume-remember" checked={remember} onCheckedChange={(v) => setRemember(v === true)} />
          <Label htmlFor="new-volume-remember">{t("volumes.new.remember")}</Label>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={dismiss.isPending} onClick={() => closeAll(remember)}>
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
