// Categories: Happy (T-ui-search-mode-icon-01), Boundary (T-ui-search-mode-icon-02), Business rule (T-ui-search-mode-icon-03)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { resetAllStores, useSearchStore } from "@/stores"
import { SearchBox } from "./SearchBox"

const toastInfo = vi.hoisted(() => vi.fn())
vi.mock("sonner", () => ({ toast: { info: toastInfo, error: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  resetAllStores()
  toastInfo.mockClear()
})

async function setup() {
  const api = createMockApi({ count: 10 })
  const listModels = vi.spyOn(api, "listLocalModels")
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SearchBox />
      </QueryClientProvider>
    </ApiProvider>
  )
  const box = screen.getByRole("combobox", { name: "검색" })
  // 검색 상자 컨테이너 = 입력 래퍼(필드)의 부모
  const container = box.parentElement?.parentElement as HTMLElement
  const button = within(container).getByRole("button", { name: "의미" })
  // 모델 목록을 받은 뒤(= 토스트 조건이 평가 가능한 상태)에 상호작용한다
  await waitFor(() => expect(listModels).toHaveBeenCalled())
  await listModels.mock.results[0].value
  return { button, user: userEvent.setup() }
}

describe("SearchBox mode icon toggle", () => {
  it("T-ui-search-mode-icon-01 [Happy] one named button inside the search box flips keyword <-> hybrid via aria-pressed", async () => {
    // Arrange
    const { button, user } = await setup()
    expect(button).toHaveAttribute("aria-pressed", "false")
    expect(useSearchStore.getState().mode).toBe("keyword")

    // Act
    await user.click(button)

    // Assert
    expect(button).toHaveAttribute("aria-pressed", "true")
    expect(useSearchStore.getState().mode).toBe("hybrid")

    // Act 2
    await user.click(button)

    // Assert 2: 접근성 이름은 상태와 무관하게 고정
    expect(button).toHaveAccessibleName("의미")
    expect(button).toHaveAttribute("aria-pressed", "false")
    expect(useSearchStore.getState().mode).toBe("keyword")
  })

  it("T-ui-search-mode-icon-02 [Boundary] there is exactly one toggle control and no radios", async () => {
    // Arrange / Act
    await setup()

    // Assert
    expect(screen.queryAllByRole("radio")).toHaveLength(0)
    expect(screen.getAllByRole("button", { name: "의미" })).toHaveLength(1)
  })

  it("T-ui-search-mode-icon-03 [Business rule] missing-model toast fires only when switching to hybrid", async () => {
    // Arrange
    const { button, user } = await setup()

    // Act: 의미로 전환
    await user.click(button)
    // Assert
    expect(toastInfo).toHaveBeenCalledTimes(1)

    // Act 2: 키워드로 복귀
    await user.click(button)
    // Assert 2: 추가 토스트 없음
    expect(toastInfo).toHaveBeenCalledTimes(1)
  })
})
