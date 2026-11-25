import { create } from "zustand"

/** 설정 페이지의 메뉴(섹션) id — 메뉴 정의는 `components/settings/settingsMenu.ts`. */
export type SettingsSection = "general" | "view" | "player" | "resume" | "folders" | "metadata" | "protection" | "data" | "ai" | "ai-local" | "plugins" | "licenses"

/** 원본 삭제·이동 확인 Dialog 대상. */
export interface OriginalsOp {
  kind: "delete" | "move"
  assetIds: number[]
}

interface ProtectState {
  /** true = 설정 화면, false = 라이브러리 화면 (화면은 둘뿐이라 라우터 없이 이 값으로 전환한다) */
  settingsOpen: boolean
  settingsSection: SettingsSection
  originals: OriginalsOp | null
  resetOpen: boolean
  /** 비밀번호 Dialog 가 열려 있으면 그 결과(인증 성공 여부)를 기다리는 resolver */
  passwordPrompt: { resolve: (ok: boolean) => void } | null
  /** 비밀번호 미설정 안내 Dialog */
  passwordGuideOpen: boolean
  /** 설정 화면으로 전환 — 섹션을 생략하면 마지막으로 보던 섹션 */
  openSettings: (section?: SettingsSection) => void
  /** 라이브러리 화면으로 복귀 */
  closeSettings: () => void
  setSettingsSection: (section: SettingsSection) => void
  askOriginals: (op: OriginalsOp | null) => void
  setResetOpen: (open: boolean) => void
  setPasswordPrompt: (p: ProtectState["passwordPrompt"]) => void
  setPasswordGuideOpen: (open: boolean) => void
}

/** 설정·보호 흐름(비밀번호 Dialog, 원본 삭제/이동, 초기화)의 열림 상태. */
export const useProtectStore = create<ProtectState>()((set) => ({
  settingsOpen: false,
  settingsSection: "general",
  originals: null,
  resetOpen: false,
  passwordPrompt: null,
  passwordGuideOpen: false,
  openSettings: (section) => set((s) => ({ settingsOpen: true, settingsSection: section ?? s.settingsSection })),
  closeSettings: () => set({ settingsOpen: false }),
  setSettingsSection: (settingsSection) => set({ settingsSection }),
  askOriginals: (originals) => set({ originals }),
  setResetOpen: (resetOpen) => set({ resetOpen }),
  setPasswordPrompt: (passwordPrompt) => set({ passwordPrompt }),
  setPasswordGuideOpen: (passwordGuideOpen) => set({ passwordGuideOpen }),
}))
