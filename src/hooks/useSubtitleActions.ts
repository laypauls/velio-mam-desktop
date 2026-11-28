import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { formatNumber } from "@/lib/format"
import type { AiSettings, LocalModel } from "@/lib/ipc-contract-ai"
import { useProtectStore } from "@/stores"
import { useAiSettings, useLocalModels } from "./useAi"
import { useAnalyzeActions } from "./useAnalyze"

/** 자막 생성을 막는 이유 — 설정에서 꺼짐 / 선택한 자막 모델 미설치 */
export type SubtitleBlock = "disabled" | "modelMissing"

/** 설정과 모델 목록으로 자막 생성을 막는 이유를 구한다. 아직 조회 전이면 막지 않는다(없음으로 단정하지 않음). */
export function subtitleBlock(settings: AiSettings | undefined, models: LocalModel[] | undefined): SubtitleBlock | null {
  if (!settings) return null
  if (!settings.localStt.enabled) return "disabled"
  if (!models) return null
  const model = models.find((m) => m.purpose === `whisper-${settings.localStt.model}`)
  return model?.state === "ready" ? null : "modelMissing"
}

/**
 * 자막 생성 요청 — `analyze(ids, ["stt"], force)` 를 감싼다. 요청 전에 설정(자막 스위치)·모델 준비를 확인하고,
 * 막히면 IPC 를 부르지 않고 안내 toast(모델 없음은 설정 이동 버튼 포함)를 보인다. 결과 toast 는 자막 문구를 쓴다.
 */
export function useSubtitleActions() {
  const { t } = useTranslation()
  const { analyze } = useAnalyzeActions()
  const openSettings = useProtectStore((s) => s.openSettings)
  const settings = useAiSettings().data
  const models = useLocalModels().data
  const block = subtitleBlock(settings, models)

  const generate = async (assetIds: number[], force = false) => {
    if (block === "disabled") {
      toast.warning(t("ai.subtitle.disabledToast"), { action: { label: t("ai.subtitle.openSettings"), onClick: () => openSettings("ai-local") } })
      return null
    }
    if (block === "modelMissing") {
      toast.warning(t("ai.subtitle.modelMissingToast"), { action: { label: t("ai.subtitle.openSettings"), onClick: () => openSettings("ai-local") } })
      return null
    }
    const res = await analyze(assetIds, ["stt"], force, { quiet: true })
    if (!res) return null
    if (res.queued > 0) {
      toast.success(t("ai.subtitle.queued", { queued: formatNumber(res.queued) }), res.skipped > 0 ? { description: t("ai.subtitle.queuedHint") } : undefined)
    } else {
      toast.info(t("ai.subtitle.nothingQueued"), { description: t("ai.subtitle.queuedHint") })
    }
    return res
  }

  return {
    generate,
    block,
    /** 버튼 활성 여부 */
    available: block === null,
    /** 비활성 이유 문구(툴팁) — 활성이면 null */
    reasonText: block === null ? null : t(`ai.subtitle.reason.${block}`),
  }
}
