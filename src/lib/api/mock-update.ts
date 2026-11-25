import type { UpdateCheck, UpdateError, UpdateStatePayload } from "@/lib/ipc-contract"
import type { MockOptions } from "./mock-ctx"
import type { UiApi } from "./types"

/** 업데이트 command mock — 기본은 "업데이트 없음". `opts.update` 로 가용 업데이트를 주입한다. */
export function createMockUpdate(opts: MockOptions): Pick<UiApi, "checkUpdate" | "installUpdate" | "onUpdateState"> {
  const handlers = new Set<(p: UpdateStatePayload) => void>()
  const emit = (p: UpdateStatePayload) => handlers.forEach((h) => h(p))
  const u = opts.update
  const fail = (code: UpdateError["code"]): UpdateError => ({ code, message: code })
  let checked = false

  return {
    checkUpdate: async (): Promise<UpdateCheck> => {
      if (opts.updateCheckError) throw fail(opts.updateCheckError)
      checked = true
      return {
        available: u !== undefined,
        currentVersion: u?.currentVersion ?? "0.1.0",
        version: u?.version ?? null,
        notes: u?.notes ?? null,
        date: u?.date ?? null,
      }
    },
    installUpdate: async () => {
      if (!checked || !u) throw fail("not_checked")
      if (opts.updateInstallError) throw fail(opts.updateInstallError)
      const total = 10_000_000
      for (const downloaded of [0, total / 2, total]) emit({ phase: "downloading", downloaded, total })
      emit({ phase: "installing", downloaded: total, total })
    },
    onUpdateState: async (handler) => {
      handlers.add(handler)
      return () => void handlers.delete(handler)
    },
  }
}
