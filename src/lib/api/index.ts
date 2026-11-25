import { createMockApi } from "./mock"
import { createTauriApi } from "./tauri"
import type { UiApi } from "./types"

export { ApiProvider, useApi } from "./context"
export type { MediaUrls, UiApi } from "./types"

/** Tauri 런타임이면 invoke 구현, 아니면(또는 `VITE_MAM_MOCK=1`) 메모리 mock (D2). */
export function createDefaultApi(): UiApi {
  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  if (inTauri && import.meta.env.VITE_MAM_MOCK !== "1") return createTauriApi()
  return createMockApi()
}
