import type { ExportProgress } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { cmdError } from "./mock-search"
import type { UiApi } from "./types"

type ReuseApi = Pick<
  UiApi,
  "pickDirectory" | "pickSaveFile" | "exportAssets" | "onExportProgress" | "copyToClipboard" | "openOriginal" | "revealInExplorer"
>

/** P2-8 재활용 mock — 내보내기(타이머로 진행률 이벤트)·클립보드·열기. */
export function createMockReuse({ rec, wait }: MockCtx): ReuseApi {
  const exportHandlers = new Set<(p: ExportProgress) => void>()
  let taskSeq = 0

  const startExport: UiApi["exportAssets"] = async (input) => {
    await wait()
    if (input.assetIds.length === 0) throw cmdError("invalid_input", "no assets")
    const taskId = `export-${++taskSeq}`
    const total = input.assetIds.length
    const failedTotal = input.assetIds.filter((id) => rec(id).avail !== "online").length
    const step = Math.max(1, Math.ceil(total / 4))
    let done = 0
    const tick = () => {
      done = Math.min(total, done + step)
      const finished = done >= total
      const p: ExportProgress = {
        taskId, done, total, finished, skipped: 0,
        failed: finished ? failedTotal : Math.min(failedTotal, done),
      }
      exportHandlers.forEach((h) => h(p))
      if (!finished) setTimeout(tick, 50)
    }
    setTimeout(tick, 50)
    return { taskId }
  }


  return {
    pickDirectory: async () => (await wait(), "D:\\Export"),
    pickSaveFile: async (name) => (await wait(), `D:\\Export\\${name ?? "file"}`),
    exportAssets: startExport,
    onExportProgress: async (h) => {
      exportHandlers.add(h)
      return () => void exportHandlers.delete(h)
    },
    copyToClipboard: async (ids) => void ids.forEach(rec),
    openOriginal: async (id) => void rec(id),
    revealInExplorer: async (id) => void rec(id),

  }
}
