// Categories: Business rule (T-ui-ai-api-01) / Validation (T-ui-ai-api-02)
import { describe, expect, it } from "vitest"
import { AI_LIMITS } from "@/lib/ipc-contract-ai"
import { findAsset, rejection } from "@/test/ai-helpers"
import { createMockApi } from "./mock"

/** 타이머를 모아 두었다가 `flush()` 로 한꺼번에 진행시키는 결정적 스케줄러. */
function manualScheduler() {
  const tasks: { fn: () => void; ms: number }[] = []
  return {
    schedule: (fn: () => void, ms: number) => void tasks.push({ fn, ms }),
    flush: () => {
      tasks.sort((a, b) => a.ms - b.ms).splice(0).forEach((t) => t.fn())
    },
  }
}

describe("mock api ai", () => {
  it("T-ui-ai-api-01 [Business rule] external analysis before consent is consent_required; after consent it queues and finishes; local stt needs no consent", async () => {
    // Arrange
    const sched = manualScheduler()
    const api = createMockApi({ count: 500, ai: { schedule: sched.schedule } })
    const image = await findAsset(api, "image")
    const video = await findAsset(api, "video")

    // Act 1: 동의 전 외부 분석
    const denied = await rejection(api.analyzeAssets([image.assetId], ["describe"], false))
    // Assert 1
    expect(denied.code).toBe("consent_required")
    expect((await api.getAssetAi(image.assetId)).status.every((s) => s.state === "none")).toBe(true)

    // Act 2: 동의했지만 키 없음 → 건너뜀
    await api.giveAiConsent()
    const noKey = await api.analyzeAssets([image.assetId], ["describe"], false)
    // Assert 2
    expect(noKey).toEqual({ queued: 0, skipped: 1 })

    // Act 3: 키 등록 후 분석 — 대기 → (진행) → 완료
    await api.setApiKey("claude", "sk-test", false)
    const queued = await api.analyzeAssets([image.assetId], ["describe"], false)
    // Assert 3
    expect(queued).toEqual({ queued: 1, skipped: 0 })
    const describeState = async () => (await api.getAssetAi(image.assetId)).status.find((s) => s.kind === "describe")?.state
    expect(await describeState()).toBe("queued")
    sched.flush()
    expect(await describeState()).toBe("done")
    const after = await api.getAssetAi(image.assetId)
    expect(after.analysis?.description).not.toBe("")
    expect((await api.listAnalysisHistory(image.assetId))[0]).toMatchObject({ kind: "describe", state: "done" })

    // Act 4: 동의 철회 후에도 로컬 STT 는 허용, 외부는 다시 거절
    await api.revokeAiConsent()
    const stt = await api.analyzeAssets([video.assetId], ["stt"], false)
    const again = await rejection(api.analyzeAssets([image.assetId], ["ocr"], false))
    // Assert 4
    expect(stt.queued).toBe(1)
    expect(again.code).toBe("consent_required")
  })

  it("T-ui-ai-api-02 [Validation] transfer/schedule values outside AI_LIMITS are invalid_input; boundaries are accepted; keys are never echoed", async () => {
    // Arrange
    const api = createMockApi({ count: 100 })
    const { imageMaxEdge: e, videoFrames: f, scheduleHour: h } = AI_LIMITS

    // Act + Assert: 범위 밖
    for (const patch of [
      { transfer: { imageMaxEdge: e.min - 1 } },
      { transfer: { imageMaxEdge: e.max + 1 } },
      { transfer: { videoFrames: f.min - 1 } },
      { transfer: { videoFrames: f.max + 1 } },
      { transfer: { videoFrames: 2.5 } },
      { schedule: { startHour: h.max + 1 } },
      { schedule: { endHour: -1 } },
    ]) {
      expect((await rejection(api.updateAiSettings(patch))).code).toBe("invalid_input")
    }
    expect((await api.getAiSettings()).transfer).toEqual({ imageMaxEdge: e.default, videoFrames: f.default, includeMetadata: true })

    // Act + Assert: 경계값
    const ok = await api.updateAiSettings({ transfer: { imageMaxEdge: e.max, videoFrames: f.min }, schedule: { startHour: h.min, endHour: h.max } })
    expect(ok.transfer).toMatchObject({ imageMaxEdge: e.max, videoFrames: f.min })
    expect(ok.schedule).toMatchObject({ startHour: h.min, endHour: h.max })

    // 동의 전 자동 분석 켜기 거절, 빈 키 거절, 키 값은 어떤 응답에도 없다
    expect((await rejection(api.updateAiSettings({ autoAnalyzeNew: true }))).code).toBe("consent_required")
    expect((await rejection(api.setApiKey("openai", "  ", true))).code).toBe("invalid_input")
    const status = await api.setApiKey("openai", "sk-SECRET-VALUE", true)
    expect(status.keyStorage).toBe("persisted")
    expect(JSON.stringify([status, await api.listAiProviders(), await api.getAiSettings()])).not.toContain("SECRET")
  })
})
