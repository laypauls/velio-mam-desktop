import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { toast } from "sonner"
import { Toaster } from "@/components/ui/sonner"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { ThemeSetting } from "@/lib/ipc-contract"
import { applyTheme, cacheTheme } from "./apply"
import { parseThemeCache, resolveTheme, type ResolvedTheme } from "./resolve"
import { useResolvedThemeStore } from "./store"
import { useOsDark } from "./useOsDark"
import { useThemeSync } from "./useThemeSync"

const setThemeMock = vi.hoisted(() => vi.fn(() => Promise.resolve()))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setTheme: setThemeMock }) }))

// 케이스 분류: Happy(해석·적용) / Boundary(캐시 없음) / External(localStorage 예외) / Concurrency(OS 변경·해제)

type Listener = (e: { matches: boolean }) => void

/** 결정적 matchMedia 스텁 — change 를 직접 발사한다. */
function stubMatchMedia(initial: boolean) {
  let matches = initial
  const listeners = new Set<Listener>()
  vi.stubGlobal("matchMedia", (query: string) => ({
    get matches() {
      return matches
    },
    media: query,
    addEventListener: (_: string, l: Listener) => listeners.add(l),
    removeEventListener: (_: string, l: Listener) => listeners.delete(l),
  }))
  return {
    set(next: boolean) {
      matches = next
      act(() => listeners.forEach((l) => l({ matches: next })))
    },
    count: () => listeners.size,
  }
}

function wrapper(api: UiApi) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>
      <ApiProvider api={api}>{children}</ApiProvider>
    </QueryClientProvider>
  )
}

function apiWithTheme(theme: ThemeSetting): UiApi {
  const api = createMockApi({ count: 10 })
  const get = api.getSettings.bind(api)
  return { ...api, getSettings: async () => ({ ...(await get()), theme }) }
}

function setTauri(on: boolean) {
  const w = window as unknown as Record<string, unknown>
  if (on) w.__TAURI_INTERNALS__ = {}
  else delete w.__TAURI_INTERNALS__
}

function denyStorage() {
  const deny = () => {
    throw new Error("denied")
  }
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(deny)
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(deny)
}

beforeEach(() => {
  document.documentElement.className = ""
  document.documentElement.removeAttribute("data-theme")
  window.localStorage.clear()
  useResolvedThemeStore.setState({ resolved: "light" })
  setThemeMock.mockClear()
})
afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  setTauri(false)
})

describe("resolveTheme", () => {
  const modes = ["system", "light", "gray", "dim", "dark"] as const
  const sds = ["dim", "dark"] as const
  const expected = (mode: (typeof modes)[number], sd: (typeof sds)[number], os: boolean): ResolvedTheme =>
    mode === "system" ? (os ? sd : "light") : mode
  for (const mode of modes)
    for (const sd of sds)
      for (const os of [false, true])
        it(`T-ui-theme-01 ${mode}/${sd}/os=${os ? "dark" : "light"}`, () => {
          expect(resolveTheme({ mode, systemDark: sd }, os)).toBe(expected(mode, sd, os))
        })
})

describe("applyTheme", () => {
  it("T-ui-theme-02 dim 은 .dark + data-theme=dim, light 는 .dark 제거", () => {
    applyTheme("dim")
    expect(document.documentElement.classList.contains("dark")).toBe(true)
    expect(document.documentElement.dataset.theme).toBe("dim")
    applyTheme("dark")
    expect(document.documentElement.dataset.theme).toBe("dark")
    applyTheme("light")
    expect(document.documentElement.classList.contains("dark")).toBe(false)
    expect(document.documentElement.dataset.theme).toBe("light")
  })

  it("T-ui-theme-20 gray 는 data-theme=gray 이며 .dark 를 붙이지 않고, dark 에서 gray 로 가면 .dark 를 제거한다", () => {
    applyTheme("dark")
    expect(document.documentElement.classList.contains("dark")).toBe(true)
    applyTheme("gray")
    expect(document.documentElement.dataset.theme).toBe("gray")
    expect(document.documentElement.classList.contains("dark")).toBe(false)
  })

  it("T-ui-theme-03같은 값이면 DOM 을 건드리지 않는다", () => {
    applyTheme("dim")
    const spy = vi.spyOn(document.documentElement, "setAttribute")
    const toggle = vi.spyOn(document.documentElement.classList, "toggle")
    applyTheme("dim")
    expect(spy).not.toHaveBeenCalled()
    expect(toggle).not.toHaveBeenCalled()
  })

  it("T-ui-theme-04 localStorage 가 던져도 apply/cache 는 던지지 않는다", () => {
    denyStorage()
    expect(() => cacheTheme({ mode: "dark", systemDark: "dim" })).not.toThrow()
    expect(() => applyTheme("dark")).not.toThrow()
    expect(document.documentElement.dataset.theme).toBe("dark")
  })

  it("T-ui-theme-05 캐시는 두 필드 JSON 으로 저장된다", () => {
    cacheTheme({ mode: "system", systemDark: "dark" })
    expect(JSON.parse(window.localStorage.getItem("mam.theme")!)).toEqual({ mode: "system", systemDark: "dark" })
  })
})

describe("useOsDark", () => {
  it("T-ui-theme-06 change 를 반영하고 언마운트 시 해제한다", () => {
    const mq = stubMatchMedia(false)
    const { result, unmount } = renderHook(() => useOsDark())
    expect(result.current).toBe(false)
    mq.set(true)
    expect(result.current).toBe(true)
    expect(mq.count()).toBe(1)
    unmount()
    expect(mq.count()).toBe(0)
  })

  it("T-ui-theme-07 matchMedia 가 없어도 false", () => {
    vi.stubGlobal("matchMedia", undefined)
    const { result } = renderHook(() => useOsDark())
    expect(result.current).toBe(false)
  })
})

describe("useThemeSync", () => {
  it("T-ui-theme-08 시스템: OS 변경 시 재적용 (다크=systemDark, 라이트=light)", async () => {
    const mq = stubMatchMedia(false)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "system", systemDark: "dark" })) })
    await waitFor(() => expect(window.localStorage.getItem("mam.theme")).not.toBeNull())
    expect(document.documentElement.dataset.theme).toBe("light")
    mq.set(true)
    expect(document.documentElement.dataset.theme).toBe("dark")
    mq.set(false)
    expect(document.documentElement.dataset.theme).toBe("light")
  })

  it("T-ui-theme-09 고정 모드는 OS 변경을 무시한다", async () => {
    const mq = stubMatchMedia(false)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "light", systemDark: "dim" })) })
    await waitFor(() => expect(window.localStorage.getItem("mam.theme")).not.toBeNull())
    mq.set(true)
    expect(document.documentElement.dataset.theme).toBe("light")
    expect(document.documentElement.classList.contains("dark")).toBe(false)
  })

  it("T-ui-theme-10 설정 로드 전에는 localStorage 캐시, 로드 후 캐시 갱신", async () => {
    stubMatchMedia(false)
    window.localStorage.setItem("mam.theme", JSON.stringify({ mode: "dark", systemDark: "dim" }))
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "dim", systemDark: "dim" })) })
    expect(document.documentElement.dataset.theme).toBe("dark")
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dim"))
    expect(JSON.parse(window.localStorage.getItem("mam.theme")!).mode).toBe("dim")
  })

  it("T-ui-theme-11 캐시·설정 모두 없으면 기본값(시스템/딤)", () => {
    stubMatchMedia(true)
    const never = { ...createMockApi({ count: 1 }), getSettings: () => new Promise<never>(() => undefined) }
    renderHook(() => useThemeSync(), { wrapper: wrapper(never) })
    expect(document.documentElement.dataset.theme).toBe("dim")
  })

  it("T-ui-theme-12 localStorage 가 던져도 설정대로 적용된다", async () => {
    stubMatchMedia(false)
    denyStorage()
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "dark", systemDark: "dim" })) })
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"))
  })

  it("T-ui-theme-13 Tauri 에서 dim 은 setTheme(dark)", async () => {
    stubMatchMedia(false)
    setTauri(true)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "dim", systemDark: "dim" })) })
    await waitFor(() => expect(setThemeMock).toHaveBeenLastCalledWith("dark"))
  })

  it("T-ui-theme-21 gray 선택 시 setTheme(light)", async () => {
    stubMatchMedia(true)
    setTauri(true)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "gray", systemDark: "dim" })) })
    await waitFor(() => expect(setThemeMock).toHaveBeenLastCalledWith("light"))
    expect(document.documentElement.dataset.theme).toBe("gray")
  })

  it("T-ui-theme-22 캐시 파싱: mode gray 허용, systemDark gray 는 거부", () => {
    expect(parseThemeCache(JSON.stringify({ mode: "gray", systemDark: "dim" }))).toEqual({ mode: "gray", systemDark: "dim" })
    expect(parseThemeCache(JSON.stringify({ mode: "system", systemDark: "gray" }))).toBeNull()
  })

  it("T-ui-theme-14Tauri 가 아니면 setTheme 을 부르지 않는다", async () => {
    stubMatchMedia(false)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "dark", systemDark: "dim" })) })
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("dark"))
    expect(setThemeMock).not.toHaveBeenCalled()
  })

  it("T-ui-theme-15 light 선택 시 setTheme(light)", async () => {
    stubMatchMedia(true)
    setTauri(true)
    renderHook(() => useThemeSync(), { wrapper: wrapper(apiWithTheme({ mode: "light", systemDark: "dim" })) })
    await waitFor(() => expect(setThemeMock).toHaveBeenLastCalledWith("light"))
  })
})

describe("Toaster", () => {
  it("T-ui-theme-16 앱 해석 테마를 sonner 에 전달한다 (dim → dark)", async () => {
    stubMatchMedia(false)
    useResolvedThemeStore.setState({ resolved: "dim" })
    render(<Toaster />)
    act(() => void toast("hello"))
    await waitFor(() => expect(document.querySelector("[data-sonner-toaster]")).not.toBeNull())
    expect(document.querySelector("[data-sonner-toaster]")!.getAttribute("data-sonner-theme")).toBe("dark")
    act(() => useResolvedThemeStore.setState({ resolved: "light" }))
    expect(document.querySelector("[data-sonner-toaster]")!.getAttribute("data-sonner-theme")).toBe("light")
    act(() => useResolvedThemeStore.setState({ resolved: "gray" }))
    expect(document.querySelector("[data-sonner-toaster]")!.getAttribute("data-sonner-theme")).toBe("light")
  })
})
