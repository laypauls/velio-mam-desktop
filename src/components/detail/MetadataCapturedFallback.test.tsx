// Categories: Happy (T-ui-capdetail-01) / Business rule (T-ui-capdetail-02, 03) / Boundary (T-ui-capdetail-03)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { formatDateTime } from "@/lib/format"
import type { AssetDetail, CapturedAtFallback } from "@/lib/ipc-contract"
import { MetadataList } from "./MetadataList"
import { useViewStore } from "@/stores"

/** "세부 정보" 는 접혀 있으므로 펼친다 (수정일은 그 안에 있다). */
async function expand() {
  await userEvent.click(await screen.findByRole("button", { name: /^세부 정보/ }))
}

async function setup(capturedAt: number | null, mode: CapturedAtFallback) {
  useViewStore.setState({ detailMetaOpen: false })
  const api = createMockApi({ count: 10 })
  await api.updateSettings({ capturedAtFallback: mode })
  const detail: AssetDetail = { ...(await api.getAssetDetail(1)), capturedAt, modifiedAt: Date.UTC(2023, 10, 2, 9, 10) }
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <MetadataList detail={detail} />
      </ApiProvider>
    </QueryClientProvider>,
  )
  return detail
}

describe("detail captured date follows capturedAtFallback", () => {
  it("T-ui-capdetail-01 [Happy] a real captured date has no fallback marker", async () => {
    const d = await setup(Date.UTC(2020, 0, 5, 3), "modified")
    expect(await screen.findByText(formatDateTime(d.capturedAt!))).toBeInTheDocument()
    expect(screen.queryByText(/\(수정일\)/)).toBeNull()
  })

  it("T-ui-capdetail-02 [Business rule] 'modified' + no captured date shows the modified date with the (수정일) marker", async () => {
    const d = await setup(null, "modified")
    expect(await screen.findByText(`${formatDateTime(d.modifiedAt)} (수정일)`)).toBeInTheDocument()
    expect(screen.getByText("촬영일")).toBeInTheDocument()
  })

  it("T-ui-capdetail-03 [Boundary] 'none' + no captured date hides the captured row (modified row stays)", async () => {
    const d = await setup(null, "none")
    await expand()
    expect(await screen.findByText(formatDateTime(d.modifiedAt))).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByText("촬영일")).toBeNull()) // 설정 로드 뒤 반영
    expect(screen.queryByText(/\(수정일\)/)).toBeNull()
  })
})
