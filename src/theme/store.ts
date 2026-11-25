import { create } from "zustand"
import type { ResolvedTheme } from "./resolve"

/** 앱이 지금 적용한 테마(해석 결과) — Toaster 등 비트리 의존 소비자용. 원천 아님. */
export const useResolvedThemeStore = create<{ resolved: ResolvedTheme; set: (r: ResolvedTheme) => void }>()((set) => ({
  resolved: "light",
  set: (resolved) => set({ resolved }),
}))
