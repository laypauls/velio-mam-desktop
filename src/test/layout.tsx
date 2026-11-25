import { QueryClient } from "@tanstack/react-query"
import { render } from "@testing-library/react"
import { vi } from "vitest"
import App from "@/App"
import type { UiApi } from "@/lib/api"
import { resetAllStores } from "@/stores"

/**
 * jsdom 은 레이아웃이 없다 — 컨테이너 크기를 고정 스텁해 가상화가 결정적으로 보이는 행만 렌더하게 한다.
 * ResizeObserver 는 아무것도 호출하지 않는 스텁(초기 크기는 offset/client 값에서 읽는다), rAF/cancel 도 함께 스텁한다.
 * 테스트 후 `vi.restoreAllMocks()` / `vi.unstubAllGlobals()` 로 되돌린다.
 */
export function stubLayout({ width = 1000, height = 600 }: { width?: number; height?: number } = {}): void {
  class NoopResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  vi.stubGlobal("ResizeObserver", NoopResizeObserver)
  vi.stubGlobal("requestAnimationFrame", () => 0)
  vi.stubGlobal("cancelAnimationFrame", () => undefined)
  // sonner(Toaster)가 prefers-color-scheme 조회에 사용
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    onchange: null,
    dispatchEvent: () => false,
  }))
  // sonner 토스트의 pointerdown 핸들러가 호출 — jsdom 에는 없다
  Element.prototype.setPointerCapture ??= () => undefined
  Element.prototype.releasePointerCapture ??= () => undefined
  vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(width)
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(height)
  vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(width)
  vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(height)
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: width,
    bottom: height,
    width,
    height,
    toJSON: () => ({}),
  })
}

/** 스토어를 초기화하고 mock API 를 주입해 앱을 렌더한다. */
export function renderApp(api: UiApi, opts: { updateCheckDelayMs?: number } = {}) {
  resetAllStores()
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  return render(<App api={api} queryClient={queryClient} updateCheckDelayMs={opts.updateCheckDelayMs} />)
}
