import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { useAnalyzeActions } from "@/hooks/useAnalyze"
import { formatNumber } from "@/lib/format"
import type { AnalysisKind } from "@/lib/ipc-contract-ai"
import { useAiStore } from "@/stores"

const KINDS: AnalysisKind[] = ["describe", "keywords", "ocr", "summary", "stt"]

function BulkAnalyzeBody({ assetIds }: { assetIds: number[] }) {
  const { t } = useTranslation()
  const close = useAiStore((s) => s.closeBulkAnalyze)
  const { analyze } = useAnalyzeActions()
  const [kinds, setKinds] = useState<Set<AnalysisKind>>(new Set(["describe", "keywords", "ocr", "stt"]))
  const [force, setForce] = useState(false)
  const [busy, setBusy] = useState(false)

  const toggle = (k: AnalysisKind, on: boolean) =>
    setKinds((prev) => {
      const next = new Set(prev)
      if (on) next.add(k)
      else next.delete(k)
      return next
    })
  const start = async () => {
    setBusy(true)
    const res = await analyze(assetIds, KINDS.filter((k) => kinds.has(k)), force)
    setBusy(false)
    if (res) close()
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t("ai.bulk.title")}</DialogTitle>
        <DialogDescription>{t("ai.bulk.description", { count: formatNumber(assetIds.length) })}</DialogDescription>
      </DialogHeader>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1 text-sm font-medium">{t("ai.bulk.kinds")}</legend>
        {KINDS.map((k) => (
          <Label key={k} className="!items-center gap-2 font-normal">
            <Checkbox checked={kinds.has(k)} onCheckedChange={(v) => toggle(k, v === true)} />
            {t(`ai.kind.${k}`)}
          </Label>
        ))}
      </fieldset>
      <div className="flex flex-col gap-1">
        <Label className="!items-center gap-2 font-normal">
          <Checkbox checked={force} onCheckedChange={(v) => setForce(v === true)} />
          {t("ai.bulk.force")}
        </Label>
        {force && <p className="text-xs break-keep text-muted-foreground">{t("ai.overwriteWarning")}</p>}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={close}>
          {t("common.cancel")}
        </Button>
        <Button disabled={busy || kinds.size === 0} onClick={() => void start()}>
          {t("ai.bulk.start")}
        </Button>
      </DialogFooter>
    </>
  )
}

/** D5: 선택 바의 "AI 분석" — 종류 선택 + 기존 결과 덮어쓰기 여부. */
export function BulkAnalyzeDialog() {
  const ids = useAiStore((s) => s.bulkAnalyzeIds)
  const close = useAiStore((s) => s.closeBulkAnalyze)
  return (
    <Dialog open={ids !== null} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-md">{ids && <BulkAnalyzeBody assetIds={ids} />}</DialogContent>
    </Dialog>
  )
}
