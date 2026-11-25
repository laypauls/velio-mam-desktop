// Categories: Business rule (T-ui-detail-11 selection bar, 12 context menu, 13 settings toggle) / Boundary (settings off by default)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { AssetContextMenu } from "@/components/gallery/AssetContextMenu"
import { SelectionBar } from "@/components/gallery/SelectionBar"
import { ProtectionTab } from "@/components/settings/ProtectionTab"
import { qk } from "@/hooks/keys"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { resetAllStores, useSelectionStore } from "@/stores"

const asset: AssetSummary = {
  assetId: 1,
  fileName: "a.jpg",
  mediaType: "image",
  availability: "online",
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1,
  capturedAt: null,
  modifiedAt: 0,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
}

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

/** 설정 조회가 끝난 뒤에 "없음" 을 단언해야 가짜 통과가 되지 않는다. */
async function mount(ui: React.ReactNode, enabled: boolean) {
  const api = createMockApi({ count: 10 })
  if (enabled) await api.updateSettings({ originalActionsEnabled: true })
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>{ui}</ApiProvider>
    </QueryClientProvider>
  )
  await queryClient.prefetchQuery({ queryKey: qk.settings, queryFn: () => api.getSettings() })
  await waitFor(() => expect(queryClient.getQueryState(qk.settings)?.status).toBe("success"))
  return { api, user: userEvent.setup() }
}

describe("original actions gate: selection bar", () => {
  const bar = <SelectionBar />
  const select = () => useSelectionStore.getState().selectAll([1, 2])

  it("T-ui-detail-11a [Business rule] setting off hides 이동/삭제 but keeps 복사/내보내기", async () => {
    // Arrange
    select()
    await mount(bar, false)
    // Assert
    const toolbar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    expect(within(toolbar).getByRole("button", { name: "복사" })).toBeInTheDocument()
    expect(within(toolbar).queryByRole("button", { name: "이동" })).not.toBeInTheDocument()
    expect(within(toolbar).queryByRole("button", { name: "삭제" })).not.toBeInTheDocument()
  })

  it("T-ui-detail-11b [Business rule] setting on shows 이동/삭제", async () => {
    // Arrange
    select()
    await mount(bar, true)
    // Assert
    const toolbar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    await waitFor(() => expect(within(toolbar).getByRole("button", { name: "이동" })).toBeInTheDocument())
    expect(within(toolbar).getByRole("button", { name: "삭제" })).toBeInTheDocument()
  })
})

describe("original actions gate: context menu", () => {
  const menu = (
    <AssetContextMenu lookup={() => asset}>
      <div data-asset-id={1}>a.jpg</div>
    </AssetContextMenu>
  )

  it("T-ui-detail-12a [Business rule] setting off hides 이동/삭제 in the context menu", async () => {
    // Arrange
    await mount(menu, false)
    // Act
    fireEvent.contextMenu(screen.getByText("a.jpg"))
    // Assert
    expect(await screen.findByRole("menuitem", { name: "내보내기" })).toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "이동" })).not.toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "삭제" })).not.toBeInTheDocument()
  })

  it("T-ui-detail-12b [Business rule] setting on shows 이동/삭제 in the context menu", async () => {
    // Arrange
    await mount(menu, true)
    // Act
    fireEvent.contextMenu(screen.getByText("a.jpg"))
    // Assert
    expect(await screen.findByRole("menuitem", { name: "이동" })).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "삭제" })).toBeInTheDocument()
  })
})

describe("original actions gate: settings toggle", () => {
  async function renderTab(enabled: boolean) {
    const { api, user } = await mount(<div />, enabled)
    const settings = await api.getSettings()
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <ApiProvider api={api}>
          <ProtectionTab settings={settings} />
        </ApiProvider>
      </QueryClientProvider>
    )
    return { api, user }
  }

  it("T-ui-detail-13 [Boundary] default is off with an explanatory note; turning it on saves originalActionsEnabled", async () => {
    // Arrange
    const { api, user } = await renderTab(false)
    const toggle = await screen.findByRole("switch", { name: "원본 파일 이동·삭제 사용" })
    // Assert: 기본 꺼짐 + 설명
    expect(toggle).toHaveAttribute("aria-checked", "false")
    expect(screen.getByText(/기본값은 꺼짐입니다/)).toBeInTheDocument()
    // Act
    await user.click(toggle)
    // Assert
    await waitFor(async () => expect((await api.getSettings()).originalActionsEnabled).toBe(true))
  })
})
