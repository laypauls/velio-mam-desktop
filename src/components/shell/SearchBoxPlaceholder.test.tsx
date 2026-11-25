// Categories: Business rule (T-ui-search-06 placeholder follows titleEditEnabled)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { resetAllStores } from "@/stores"
import { SearchBox } from "./SearchBox"

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

async function renderBox(titleEditEnabled: boolean) {
  const api = createMockApi({ count: 10 })
  await api.updateSettings({ titleEditEnabled })
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SearchBox />
      </QueryClientProvider>
    </ApiProvider>
  )
  return screen.getByRole("combobox", { name: "검색" })
}

describe("SearchBox placeholder", () => {
  it("T-ui-search-06 [Business rule] shows the title-aware placeholder only when title edit is enabled", async () => {
    // Arrange + Act: ON
    const on = await renderBox(true)
    // Assert: 설정을 받기 전에는 파일명 문구였다가 받은 뒤 바뀐다
    await vi.waitFor(() => expect(on).toHaveAttribute("placeholder", "제목·파일명·태그·메모 검색"))
  })

  it("T-ui-search-06b [Business rule] keeps the file-name placeholder when title edit is disabled and before settings load", async () => {
    // Arrange + Act
    const off = await renderBox(false)
    // Assert: 로드 전(즉시)
    expect(off).toHaveAttribute("placeholder", "파일명·태그·메모 검색")
    // 로드 후에도 유지
    await vi.waitFor(() => expect(off).toHaveAttribute("placeholder", "파일명·태그·메모 검색"))
  })
})
