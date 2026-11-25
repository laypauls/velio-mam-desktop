import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { AI_LIMITS, type AiSettings } from "@/lib/ipc-contract-ai"
import { NumberField } from "../NumberField"
import { SettingRow } from "../SettingRow"
import type { AiCtl } from "./types"

type ExternalKind = AiSettings["externalKinds"][number]
const KINDS: ExternalKind[] = ["describe", "keywords", "ocr", "summary"]

/** 외부 AI 로 보내는 범위(이미지 긴 변·영상 프레임 수·메타데이터)와 분석 종류. 절대 경로는 보내지 않는다. */
export function TransferSection({ ctl }: { ctl: AiCtl }) {
  const { t } = useTranslation()
  const { settings, patch, commit } = ctl
  const { transfer, externalKinds } = settings
  const toggleKind = (k: ExternalKind, on: boolean) =>
    patch({ externalKinds: on ? [...externalKinds, k] : externalKinds.filter((x) => x !== k) })
  return (
    <section aria-label={t("ai.transfer.section")} className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-6">
        <NumberField
          label={t("ai.transfer.imageEdge")}
          unit={t("settings.unit.px")}
          value={transfer.imageMaxEdge}
          min={AI_LIMITS.imageMaxEdge.min}
          max={AI_LIMITS.imageMaxEdge.max}
          onCommit={(imageMaxEdge) => commit({ transfer: { imageMaxEdge } })}
        />
        <NumberField
          label={t("ai.transfer.frames")}
          unit={t("settings.unit.frames")}
          value={transfer.videoFrames}
          min={AI_LIMITS.videoFrames.min}
          max={AI_LIMITS.videoFrames.max}
          onCommit={(videoFrames) => commit({ transfer: { videoFrames } })}
        />
      </div>
      <SettingRow label={t("ai.transfer.metadata")} description={t("ai.transfer.metadataHint")}>
        <Switch aria-label={t("ai.transfer.metadata")} checked={transfer.includeMetadata} onCheckedChange={(includeMetadata) => patch({ transfer: { includeMetadata } })} />
      </SettingRow>
      <fieldset className="flex flex-wrap gap-4">
        <legend className="mb-1 text-sm font-medium">{t("ai.transfer.kinds")}</legend>
        {KINDS.map((k) => (
          <Label key={k} className="!items-center gap-2 font-normal">
            <Checkbox checked={externalKinds.includes(k)} onCheckedChange={(v) => toggleKind(k, v === true)} />
            {t(`ai.kind.${k}`)}
          </Label>
        ))}
      </fieldset>
    </section>
  )
}
