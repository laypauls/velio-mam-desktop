// Categories: Happy (T-ui-filter-01) / Validation (T-ui-filter-02) / Boundary (T-ui-filter-03)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import * as ipc from "@/lib/ipc-metadata"
import { resetAllStores, useSearchStore } from "@/stores"
import { FilterPopover } from "./FilterPopover"

vi.mock("@/lib/ipc-metadata")

beforeEach(() => {
  resetAllStores()
  vi.mocked(ipc.listMetadataFields).mockResolvedValue([]) // 사용자 필드 없음 — 섹션은 그려지지 않는다
})
afterEach(() => vi.restoreAllMocks())

async function open() {
  const user = userEvent.setup()
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <FilterPopover />
    </QueryClientProvider>
  )
  await user.click(screen.getByRole("button", { name: "필터" }))
  const field = (name: string) => screen.getByRole<HTMLInputElement>("spinbutton", { name })
  const date = (name: string) => screen.getByLabelText<HTMLInputElement>(name)
  return { user, field, date }
}
const filters = () => useSearchStore.getState().filters

describe("FilterPopover ranges", () => {
  it("T-ui-filter-01 [Happy] date, size (MB to bytes) and duration (seconds to ms) ranges map to SearchFilters", async () => {
    // Arrange
    const { user, field, date } = await open()
    // Act
    fireEvent.change(date("촬영일 시작"), { target: { value: "2026-10-01" } })
    fireEvent.change(date("촬영일 끝"), { target: { value: "2026-10-31" } })
    await user.type(field("최소 크기 (MB)"), "2")
    await user.type(field("최대 크기 (MB)"), "10.5")
    await user.type(field("최소 길이 (초)"), "30")
    await user.type(field("최대 길이 (초)"), "90")
    // Assert
    const f = filters()
    expect(f.capturedFrom).toBe(new Date(2026, 9, 1).getTime())
    expect(f.capturedTo).toBe(new Date(2026, 9, 31).getTime() + 86_400_000 - 1) // 끝 날짜는 그날 마지막 ms
    expect(f.sizeMin).toBe(2 * 1024 * 1024)
    expect(f.sizeMax).toBe(Math.round(10.5 * 1024 * 1024))
    expect(f.durationMinMs).toBe(30_000)
    expect(f.durationMaxMs).toBe(90_000)
  })

  it("T-ui-filter-02 [Validation] min > max shows an inline aria-invalid error and does not apply; fixing it applies", async () => {
    // Arrange
    const { user, field } = await open()
    await user.type(field("최소 크기 (MB)"), "5")
    await user.type(field("최대 크기 (MB)"), "9")
    expect(filters()).toMatchObject({ sizeMin: 5 * 1024 * 1024, sizeMax: 9 * 1024 * 1024 })
    // Act: 최대를 최소보다 작게 (9 -> 3)
    await user.clear(field("최대 크기 (MB)"))
    await user.type(field("최대 크기 (MB)"), "3")
    // Assert: 오류 표시, 3 은 적용 안 됨
    const alert = screen.getByRole("alert")
    expect(alert).toHaveTextContent("최소값이 최대값보다 클 수 없습니다.")
    for (const name of ["최소 크기 (MB)", "최대 크기 (MB)"]) {
      expect(field(name)).toHaveAttribute("aria-invalid", "true")
      expect(field(name)).toHaveAccessibleDescription("최소값이 최대값보다 클 수 없습니다.")
    }
    expect(filters().sizeMax).toBeNull() // clear 로 비워진 마지막 유효 상태 — 역전된 3 은 적용되지 않았다
    expect(filters().sizeMin).toBe(5 * 1024 * 1024)
    // Act: 바로잡기 (3 -> 30)
    await user.clear(field("최대 크기 (MB)"))
    await user.type(field("최대 크기 (MB)"), "30")
    // Assert
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    expect(field("최대 크기 (MB)")).not.toHaveAttribute("aria-invalid")
    expect(filters().sizeMax).toBe(30 * 1024 * 1024)
  })

  it("T-ui-filter-03 [Boundary] reversed dates are rejected, equal bounds and cleared fields are accepted, reset clears the inputs", async () => {
    // Arrange
    const { user, field, date } = await open()
    // Act: 시작 > 끝
    fireEvent.change(date("촬영일 끝"), { target: { value: "2026-10-01" } })
    fireEvent.change(date("촬영일 시작"), { target: { value: "2026-10-05" } })
    // Assert
    expect(screen.getByRole("alert")).toBeInTheDocument()
    expect(date("촬영일 시작")).toHaveAttribute("aria-invalid", "true")
    expect(filters().capturedFrom).toBeNull()
    // Act: 같은 날은 유효
    fireEvent.change(date("촬영일 시작"), { target: { value: "2026-10-01" } })
    // Assert
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    expect(filters().capturedFrom).toBe(new Date(2026, 9, 1).getTime())
    // Act: 길이 최대만 입력(최소 비움) 후 초기화
    await user.type(field("최대 길이 (초)"), "0")
    expect(filters().durationMaxMs).toBe(0)
    await user.click(screen.getByRole("button", { name: "필터 초기화" }))
    // Assert
    expect(filters().durationMaxMs).toBeNull()
    expect(field("최대 길이 (초)")).toHaveValue(null)
    expect(date("촬영일 시작")).toHaveValue("")
  })
})
