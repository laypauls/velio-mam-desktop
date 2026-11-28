// Categories: Happy (T-ui-ipc-subtitle-01 command name, 03 mock queues) / Business rule (T-ui-ipc-subtitle-02 mock rejects when off, 04 no consent) / External (T-ui-ipc-subtitle-05 reject propagates)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import { generateSubtitlesAll } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

beforeEach(() => vi.mocked(invoke).mockReset())

describe("generate_subtitles_all", () => {
  it("T-ui-ipc-subtitle-01 [Happy] wrapper and Tauri api invoke generate_subtitles_all with no args and return QueueResult", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ queued: 5, skipped: 0 })
    // Act
    const direct = await generateSubtitlesAll()
    const viaApi = await createTauriApi().generateSubtitlesAll()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["generate_subtitles_all"], ["generate_subtitles_all"]])
    expect(direct).toEqual({ queued: 5, skipped: 0 })
    expect(viaApi).toEqual({ queued: 5, skipped: 0 })
  })

  it("T-ui-ipc-subtitle-02 [Business rule] mock rejects with invalid_input when subtitles are switched off", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    await api.updateAiSettings({ localStt: { enabled: false } })
    // Act / Assert
    await expect(api.generateSubtitlesAll()).rejects.toMatchObject({ code: "invalid_input" })
  })

  it("T-ui-ipc-subtitle-03 [Happy] mock queues video/audio and a second call queues nothing new", async () => {
    // Arrange
    const api = createMockApi({ count: 30, ai: { schedule: () => undefined } })
    // Act
    const first = await api.generateSubtitlesAll()
    const second = await api.generateSubtitlesAll()
    // Assert
    expect(first.queued).toBeGreaterThan(0)
    expect(second.queued).toBe(0)
  })

  it("T-ui-ipc-subtitle-04 [Business rule] mock accepts autoSubtitleNew without external consent", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    // Act
    const after = await api.updateAiSettings({ autoSubtitleNew: true })
    // Assert
    expect(after.consent.given).toBe(false)
    expect(after.autoSubtitleNew).toBe(true)
  })

  it("T-ui-ipc-subtitle-05 [External] a rejected command propagates", async () => {
    // Arrange
    vi.mocked(invoke).mockRejectedValueOnce({ code: "invalid_input", message: "off" })
    // Act / Assert
    await expect(generateSubtitlesAll()).rejects.toMatchObject({ code: "invalid_input" })
  })
})
