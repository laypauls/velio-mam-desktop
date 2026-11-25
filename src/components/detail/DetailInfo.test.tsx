// Categories: Happy (T-ui-detail-01, 02, 04b, 09) / Boundary (T-ui-detail-03, 04, 07b) / Business rule (T-ui-detail-02b, 05..08, 10, 11, 11b, 12) / External (T-ui-detail-01b clipboard failure)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetDetail } from "@/lib/ipc-contract"
import { resetAllStores, useProtectStore, useViewStore } from "@/stores"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AssetInfo } from "./AssetInfo"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

type Api = ReturnType<typeof createMockApi>

/** ResizeObserver 스텁 — 콜백은 `fireResize` 로 수동 호출한다 (실제 RO 콜백을 기다리지 않는다). */
const observers = new Set<{ cb: ResizeObserverCallback; disconnected: boolean }>()
class StubResizeObserver {
  private rec: { cb: ResizeObserverCallback; disconnected: boolean }
  constructor(cb: ResizeObserverCallback) {
    this.rec = { cb, disconnected: false }
    observers.add(this.rec)
  }
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {
    this.rec.disconnected = true
    observers.delete(this.rec)
  }
}
function fireResize(width: number) {
  act(() => {
    for (const o of [...observers]) o.cb([{ contentRect: { width } } as ResizeObserverEntry], {} as ResizeObserver)
  })
}
/** 작업 줄 폭: 인라인 아이콘 버튼 n 개 + ⋯ 가 들어가는 폭 = (n+1)*32 + n*8 (항목이 더 많을 때) */
const NARROW_ONLY_MORE = 40 // ⋯ 만 남는 폭
const NARROW_THREE_INLINE = 152 // 인라인 3개 + ⋯

async function setup(
  patch: Partial<AssetDetail> = {},
  settings: { originalActionsEnabled?: boolean } = {},
  rowWidth?: number
) {
  observers.clear()
  vi.stubGlobal("ResizeObserver", StubResizeObserver)
  const api: Api = createMockApi({ count: 10 })
  if (settings.originalActionsEnabled) await api.updateSettings({ originalActionsEnabled: true })
  const base = await api.getAssetDetail(1)
  const detail: AssetDetail = {
    ...base,
    status: "active",
    availability: "online",
    tags: [],
    collections: [],
    duplicates: [],
    previousVersions: [],
    newerVersion: null,
    codec: null,
    audioCodec: null,
    fps: null,
    bitrate: null,
    camera: null,
    volumeLabel: null,
    extra: {},
    ...patch,
  }
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <TooltipProvider>
          <AssetInfo data={detail} />
        </TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  if (rowWidth !== undefined) fireResize(rowWidth)
  const user = userEvent.setup()
  // userEvent.setup() 이 clipboard 스텁을 붙이므로 그 뒤에 덮어쓴다
  const writeText = vi.fn().mockResolvedValue(undefined)
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true })
  return { api, detail, user, writeText }
}

describe("detail info: copy", () => {
  it("T-ui-detail-01 [Happy] the file name copy button writes the file name (not the path) and toasts", async () => {
    // Arrange
    const { detail, user, writeText } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "파일명 복사" }))
    // Assert
    expect(writeText).toHaveBeenCalledWith(detail.fileName)
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("파일명을 복사했습니다."))
  })

  it("T-ui-detail-01b [External] clipboard failure shows an error toast", async () => {
    // Arrange
    const { user, writeText } = await setup()
    writeText.mockRejectedValue(new Error("denied"))
    // Act
    await user.click(screen.getByRole("button", { name: "파일명 복사" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("클립보드에 복사하지 못했습니다."))
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-detail-02 [Happy] the path copy button writes the full path", async () => {
    // Arrange
    const { detail, user, writeText } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "전체 경로 복사" }))
    // Assert
    expect(writeText).toHaveBeenCalledWith(detail.path)
  })

  it("T-ui-detail-02b [Business rule] favorite heart is a pressed-state toggle and the old 복사 button is gone", async () => {
    // Arrange
    const { detail } = await setup({ favorite: true })
    // Assert
    expect(screen.getByRole("button", { name: "즐겨찾기" })).toHaveAttribute("aria-pressed", "true")
    expect(screen.queryByRole("button", { name: "복사" })).not.toBeInTheDocument()
    expect(screen.getByRole("heading", { level: 2, name: detail.fileName })).toBeInTheDocument()
  })
})

describe("detail info: empty states", () => {
  it("T-ui-detail-03 [Boundary] empty tags/collections show no em dash; collections explain emptiness", async () => {
    // Arrange + Act
    await setup()
    // Assert
    expect(screen.queryByText("—")).not.toBeInTheDocument()
    expect(screen.queryByRole("list", { name: "태그" })).not.toBeInTheDocument()
    expect(screen.getByText("어느 컬렉션에도 없습니다")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "컬렉션 편집" })).toBeInTheDocument()
  })

  it("T-ui-detail-04 [Boundary] related files are hidden when there are no duplicates or previous versions", async () => {
    // Arrange + Act
    await setup()
    // Assert
    expect(screen.queryByRole("button", { name: /관련 파일/ })).not.toBeInTheDocument()
  })

  it("T-ui-detail-04b [Happy] duplicates and previous versions collapse into one 관련 파일 line with counts", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const [a, b, c] = await Promise.all([1, 2, 3].map((id) => api.getAssetDetail(id)))
    const { user } = await setup({ duplicates: [b, c], previousVersions: [a] })
    // Assert: collapsed — lists not rendered yet
    const trigger = screen.getByRole("button", { name: "관련 파일 · 중복본 2 · 이전 버전 1" })
    expect(trigger).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByText("중복본 (2)")).not.toBeInTheDocument()
    // Act
    await user.click(trigger)
    // Assert
    expect(await screen.findByText("중복본 (2)")).toBeInTheDocument()
    expect(screen.getByText("이전 버전 (1)")).toBeInTheDocument()
  })
})

describe("detail info: availability", () => {
  it("T-ui-detail-05 [Business rule] online shows no status row, badge or banner", async () => {
    // Arrange + Act
    await setup()
    // Assert
    expect(screen.queryByText("정상")).not.toBeInTheDocument()
    expect(screen.queryByText("연결됨")).not.toBeInTheDocument()
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
    expect(screen.queryByRole("note")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "열기" })).toBeEnabled()
  })

  it("T-ui-detail-06 [Business rule] offline shows a banner with cause and remedy and disables open/reveal", async () => {
    // Arrange + Act
    await setup({ availability: "offline", volumeLabel: "외장 SSD" })
    // Assert
    const banner = screen.getByRole("status")
    expect(banner).toHaveTextContent("저장 장치가 연결되어 있지 않습니다")
    expect(banner).toHaveTextContent("'외장 SSD' 을(를) 연결하면")
    expect(screen.getByRole("button", { name: "열기" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "탐색기에서 보기" })).toBeDisabled()
  })

  it("T-ui-detail-11 [Business rule] the 크게 보기 button is enabled for online and opens the preview", async () => {
    // Arrange
    const { user } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "크게 보기" }))
    // Assert
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-detail-11b [Business rule] the 크게 보기 button is disabled for offline", async () => {
    // Arrange + Act
    await setup({ availability: "offline" })
    // Assert
    expect(screen.getByRole("button", { name: "크게 보기" })).toBeDisabled()
  })

  it("T-ui-detail-12 [Business rule] missing disables 크게 보기 and stays closed", async () => {
    // Arrange
    await setup({ availability: "missing" })
    // Act + Assert
    expect(screen.getByRole("button", { name: "크게 보기" })).toBeDisabled()
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-detail-07 [Business rule] missing shows a banner whose 다시 스캔 rescans the asset's folder", async () => {
    // Arrange
    const { api, user } = await setup({ availability: "missing", folderId: 1 })
    const rescan = vi.spyOn(api, "rescanFolder")
    // Assert
    expect(screen.getByRole("status")).toHaveTextContent("원래 위치에 파일이 없습니다")
    expect(screen.getByRole("button", { name: "열기" })).toBeDisabled()
    // Act
    await user.click(screen.getByRole("button", { name: "다시 스캔" }))
    // Assert
    await waitFor(() => expect(rescan).toHaveBeenCalledWith(1))
  })

  it("T-ui-detail-07b [Boundary] missing without a known folder has no rescan button", async () => {
    // Arrange + Act
    await setup({ availability: "missing", folderId: null })
    // Assert
    expect(screen.getByRole("status")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "다시 스캔" })).not.toBeInTheDocument()
  })

  it("T-ui-detail-08 [Business rule] superseded shows the note banner with a button to the newer version", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const newer = await api.getAssetDetail(2)
    await setup({ status: "superseded", newerVersion: newer })
    // Assert
    const note = screen.getByRole("note")
    expect(within(note).getByText("이 파일은 더 새로운 버전으로 대체되었습니다.")).toBeInTheDocument()
    expect(within(note).getByRole("button", { name: /새 버전 보기/ })).toBeInTheDocument()
  })
})

describe("detail info: icon-only action buttons", () => {
  it("T-ui-detail-15 [Happy] enabled 열기 is icon-only with an accessible name and shows its label tooltip on focus", async () => {
    // Arrange
    await setup()
    const open = screen.getByRole("button", { name: "열기" })
    // Assert: 보이는 텍스트 없음
    expect(open).toBeEnabled()
    expect(open).toHaveTextContent("")
    // Act
    act(() => open.focus())
    // Assert
    expect(await screen.findByRole("tooltip")).toHaveTextContent("열기")
  })

  it("T-ui-detail-15b [Happy] the ⋯ trigger shows a tooltip on hover and still opens its menu", async () => {
    // Arrange
    const { user } = await setup({}, {}, NARROW_ONLY_MORE)
    const more = screen.getByRole("button", { name: "더 보기" })
    // Act
    await user.hover(more)
    // Assert
    expect(await screen.findByRole("tooltip")).toHaveTextContent("더 보기")
    // Act
    await user.click(more)
    // Assert
    expect(await screen.findByRole("menuitem", { name: "내보내기…" })).toBeInTheDocument()
  })

  it("T-ui-detail-18 [Happy] wide row (or not measured yet): every action is inline and there is no ⋯ button", async () => {
    // Arrange + Act
    await setup({}, {}, 1000)
    // Assert: 원본 이동·삭제 설정이 꺼져 있어 4개
    for (const name of ["열기", "탐색기에서 보기", "파일을 클립보드에 복사", "내보내기…"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument()
    }
    expect(screen.queryByRole("button", { name: "더 보기" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "원본 이동…" })).not.toBeInTheDocument()
  })

  it("T-ui-detail-19 [Boundary] narrow row with 원본 이동/삭제 on: first 3 inline, the rest (with a separator) live in ⋯", async () => {
    // Arrange
    const { user } = await setup({}, { originalActionsEnabled: true }, NARROW_THREE_INLINE)
    // Assert: 인라인 3개 + ⋯ (설정은 비동기로 로드된다)
    const more = await screen.findByRole("button", { name: "더 보기" })
    expect(screen.getByRole("button", { name: "열기" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "파일을 클립보드에 복사" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "내보내기…" })).not.toBeInTheDocument()
    // Act
    await user.click(more)
    // Assert
    expect(await screen.findByRole("menuitem", { name: "내보내기…" })).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "원본 이동…" })).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "원본 삭제…" })).toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "열기" })).not.toBeInTheDocument()
    expect(screen.getAllByRole("separator")).toHaveLength(1)
  })

  it("T-ui-detail-20 [Boundary] when only 원본 이동/삭제 overflow, the menu has no leading separator", async () => {
    // Arrange: 6개 중 4개 인라인 + ⋯ = 5칸 = 200px
    const { user } = await setup({}, { originalActionsEnabled: true }, 200)
    // Act
    await user.click(await screen.findByRole("button", { name: "더 보기" }))
    // Assert
    expect(await screen.findByRole("menuitem", { name: "원본 이동…" })).toBeInTheDocument()
    expect(screen.queryByRole("separator")).not.toBeInTheDocument()
  })

  it("T-ui-detail-16[Business rule] offline: focusing the disabled button's wrapper shows label then reason", async () => {
    // Arrange
    const { user } = await setup({ availability: "offline" })
    const reveal = screen.getByRole("button", { name: "탐색기에서 보기" })
    expect(reveal).toBeDisabled()
    // Act
    const wrapper = reveal.parentElement as HTMLElement
    await user.hover(wrapper)
    // Assert
    const tip = await screen.findByRole("tooltip")
    expect(tip).toHaveTextContent("탐색기에서 보기")
    expect(tip).toHaveTextContent("저장 장치")
  })

  it("T-ui-detail-17 [Boundary] ReasonTooltip without label keeps reason-only behavior (크게 보기 offline)", async () => {
    // Arrange
    const { user } = await setup({ availability: "offline" })
    const enlarge = screen.getByRole("button", { name: "크게 보기" })
    // Act
    await user.hover(enlarge.parentElement as HTMLElement)
    // Assert: 이유만 있고 라벨 줄 없음
    const tip = await screen.findByRole("tooltip")
    expect(tip).toHaveTextContent("저장 장치")
    expect(tip).not.toHaveTextContent("크게 보기")
  })
})

describe("detail info: metadata and actions", () => {
  it("T-ui-detail-09 [Happy] summary stays visible; codec etc. live in a collapsed 세부 정보 that opens on demand", async () => {
    // Arrange
    const { user } = await setup({ codec: "H.264", camera: "Sony A7", width: 3840, height: 2160, durationMs: 134_000 })
    // Assert: 요약 한 줄, 유형 행 없음, 접힌 상태
    expect(screen.getByText(/3,840 × 2,160/)).toBeInTheDocument()
    expect(screen.queryByText("유형")).not.toBeInTheDocument()
    const toggle = screen.getByRole("button", { name: /^세부 정보 \(\d+\)$/ })
    expect(toggle).toHaveAttribute("aria-expanded", "false")
    expect(screen.queryByText("H.264")).not.toBeInTheDocument()
    // Act
    await user.click(toggle)
    // Assert
    expect(await screen.findByText("H.264")).toBeInTheDocument()
    expect(screen.getByText("Sony A7")).toBeInTheDocument()
  })

  it("T-ui-detail-10 [Business rule] the ⋯ menu offers file copy and export, but no 원본 이동/삭제 while the setting is off", async () => {
    // Arrange
    const { user } = await setup({}, {}, NARROW_ONLY_MORE)
    // Act
    await user.click(screen.getByRole("button", { name: "더 보기" }))
    // Assert
    expect(await screen.findByRole("menuitem", { name: "파일을 클립보드에 복사" })).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "내보내기…" })).toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "원본 이동…" })).not.toBeInTheDocument()
    expect(screen.queryByRole("menuitem", { name: "원본 삭제…" })).not.toBeInTheDocument()
  })

  it("T-ui-detail-10b [Business rule] setting on shows 원본 이동/삭제 in the ⋯ menu, disabled for offline assets", async () => {
    // Arrange
    const { user } = await setup({ availability: "offline" }, { originalActionsEnabled: true }, NARROW_ONLY_MORE)
    // Act
    await user.click(screen.getByRole("button", { name: "더 보기" }))
    // Assert (설정은 비동기로 로드된다)
    const move = await screen.findByRole("menuitem", { name: "원본 이동…" })
    expect(move).toHaveAttribute("aria-disabled", "true")
    expect(screen.getByRole("menuitem", { name: "원본 삭제…" })).toHaveAttribute("aria-disabled", "true")
  })

  it("T-ui-detail-10c [Business rule] setting on and online: 원본 이동 asks for confirmation instead of running directly", async () => {
    // Arrange
    const { user } = await setup({}, { originalActionsEnabled: true }, NARROW_ONLY_MORE)
    // Act
    await user.click(screen.getByRole("button", { name: "더 보기" }))
    await user.click(await screen.findByRole("menuitem", { name: "원본 이동…" }))
    // Assert: 확인 흐름 상태가 열린다 (비밀번호는 그 뒤 단계)
    expect(useProtectStore.getState().originals).toMatchObject({ kind: "move", assetIds: [1] })
  })
})
