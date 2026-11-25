import { FolderOpen } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { useOriginalsRun } from "@/hooks/useOriginals"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { useProtectStore, type OriginalsOp } from "@/stores"

function OriginalsBody({ op, run }: { op: OriginalsOp; run: ReturnType<typeof useOriginalsRun> }) {
  const { t } = useTranslation()
  const api = useApi()
  const ask = useProtectStore((s) => s.askOriginals)
  const [dest, setDest] = useState<string | null>(null)
  const [pickError, setPickError] = useState<string | null>(null)
  const count = formatNumber(op.assetIds.length)
  const isMove = op.kind === "move"

  const pick = async () => {
    setPickError(null)
    try {
      const dir = await api.pickDirectory()
      if (dir !== null) setDest(dir)
    } catch (e) {
      setPickError(t(errorKey(e)))
    }
  }

  return (
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{t(`originals.${op.kind}.title`, { count })}</AlertDialogTitle>
        <AlertDialogDescription className="break-keep">{t(`originals.${op.kind}.description`)}</AlertDialogDescription>
      </AlertDialogHeader>
      {isMove && (
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("originals.move.destination")}</span>
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={() => void pick()}>
              <FolderOpen aria-hidden="true" />
              {t("export.pick")}
            </Button>
            <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground" title={dest ?? undefined}>
              {dest ?? t("export.noDestination")}
            </span>
          </div>
          {pickError !== null && (
            <p role="alert" className="text-xs text-destructive">
              {pickError}
            </p>
          )}
        </div>
      )}
      <AlertDialogFooter>
        <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
        <AlertDialogAction
          variant={isMove ? "default" : "destructive"}
          disabled={isMove && dest === null}
          onClick={() => {
            ask(null)
            void run(op.kind, op.assetIds, dest ?? undefined)
          }}
        >
          {t(`originals.${op.kind}.confirm`)}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  )
}

/** 원본 삭제·이동 확인 (AlertDialog) → 확인하면 보호 흐름(비밀번호)을 거쳐 실행한다. */
export function OriginalsDialog() {
  const run = useOriginalsRun() // 확인 뒤 본문이 닫혀도 실행이 이어지도록 항상 마운트된 쪽에서 보관
  const op = useProtectStore((s) => s.originals)
  const ask = useProtectStore((s) => s.askOriginals)
  return (
    <AlertDialog open={op !== null} onOpenChange={(o) => !o && ask(null)}>
      {op && <OriginalsBody op={op} run={run} />}
    </AlertDialog>
  )
}
