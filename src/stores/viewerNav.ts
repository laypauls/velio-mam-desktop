import { create } from "zustand"

/** 갤러리가 보기 영역에 내주는 "현재 결과 순서에서 이전/다음 항목으로 이동" — 갤러리 화살표 키와 같은 경로를 쓴다 */
export interface GalleryStepper {
  canPrev: boolean
  canNext: boolean
  step: (dir: 1 | -1) => void
}

interface ViewerNavState {
  /** 갤러리가 마운트돼 있고 열린 항목 앞뒤로 이동할 곳이 있을 때만 값이 있다 */
  stepper: GalleryStepper | null
  setStepper: (stepper: GalleryStepper | null) => void
}

export const useViewerNavStore = create<ViewerNavState>((set) => ({
  stepper: null,
  setStepper: (stepper) => set({ stepper }),
}))
