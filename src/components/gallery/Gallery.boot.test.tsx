// Categories: External failure (T-ui-boot-01 색인 준비 중 응답) / Business rule (-02 다른 오류는 그대로 표면화) /
// External failure (-03 렌더 예외 시 빈 화면 방지)
import { screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { CommandError } from "@/lib/ipc-contract"
import { INDEX_RETRY_MS } from "@/hooks/useSearchAssets"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

// search_service.rs 의 NotReady 변환 결과와 같은 형태 (code = index_not_ready, 고정 문구)
const NOT_READY: CommandError = { code: "index_not_ready", message: "search index is not ready" }

describe("first window while the search index is being built", () => {
  it("T-ui-boot-01 [External] shows the indexing state instead of an error, then the gallery once the index is ready", async () => {
    // Arrange: 첫 두 번은 색인 준비 중, 이후 정상 응답
    stubLayout()
    const api = createMockApi({ count: 300 })
    const real = api.searchIndex.bind(api)
    let calls = 0
    vi.spyOn(api, "searchIndex").mockImplementation(async (r) => {
      calls += 1
      if (calls <= 2) throw NOT_READY
      return real(r)
    })

    // Act
    renderApp(api)

    // Assert: 오류 알림(role=alert) 없이 "준비 중" 상태가 보인다
    const status = await screen.findByText("검색 색인을 준비하고 있습니다")
    expect(status.closest("[role=status]")?.getAttribute("aria-busy")).toBe("true")
    expect(screen.queryByRole("alert")).toBeNull()
    // 사용자 조작 없이 다시 물어 목록이 나타난다
    const options = await screen.findAllByRole("option", {}, { timeout: INDEX_RETRY_MS * 4 })
    expect(options.length).toBeGreaterThan(0)
    expect(screen.queryByText("검색 색인을 준비하고 있습니다")).toBeNull()
    expect(calls).toBeGreaterThanOrEqual(3)
  })

  it("T-ui-boot-02 [Business rule] other internal errors still show the inline error with retry", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "searchIndex").mockRejectedValue({ code: "internal", message: "internal error" } satisfies CommandError)

    // Act
    renderApp(api)

    // Assert
    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(screen.getByRole("button", { name: "다시 시도" })).toBeTruthy()
    expect(screen.queryByText("검색 색인을 준비하고 있습니다")).toBeNull()
  })

  it("T-ui-boot-03 [External] a render exception shows a recoverable fallback instead of a blank window", async () => {
    // Arrange: 형식이 어긋난 색인 응답(구간이 null)이 렌더 중 예외를 만든다 — 경계가 없으면 루트가 통째로 사라진다
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "searchIndex").mockResolvedValue({ total: 3, generation: 1, kind: "month", sections: [null] } as never)
    vi.spyOn(console, "error").mockImplementation(() => undefined)

    // Act
    const { container } = renderApp(api)

    // Assert
    await waitFor(() => expect(screen.getByText("화면을 표시하지 못했습니다")).toBeTruthy())
    expect(screen.getByRole("button", { name: "다시 불러오기" })).toBeTruthy()
    expect(container.childElementCount).toBeGreaterThan(0)
  })
})

describe("results refresh state", () => {
  it("T-ui-boot-05 [Happy] the list is aria-busy while new results are pending and settles when they arrive", async () => {
    // Arrange: 첫 페이지는 바로, 질의가 바뀐 요청은 수동으로 풀어 준다
    stubLayout()
    const api = createMockApi({ count: 300 })
    const real = api.searchIndex.bind(api)
    let release: () => void = () => undefined
    vi.spyOn(api, "searchIndex").mockImplementation(async (r) => {
      if (r.query !== "") await new Promise<void>((res) => (release = res))
      return real(r)
    })
    renderApp(api)
    const list = await screen.findByRole("listbox")
    expect(list.getAttribute("aria-busy")).toBeNull()

    // Act
    const box = screen.getByRole("combobox", { name: "검색" })
    await userEvent.type(box, "a{Enter}")

    // Assert
    await waitFor(() => expect(screen.getByRole("listbox").getAttribute("aria-busy")).toBe("true"))
    release()
    await waitFor(() => expect(screen.getByRole("listbox").getAttribute("aria-busy")).toBeNull())
  })
})
