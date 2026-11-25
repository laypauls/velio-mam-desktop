// Categories: Happy (T-ui-cell-01, 03, 06, 14) / Boundary (T-ui-cell-02, 04, 05) / Business rule (T-ui-cell-07..13)
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { formatBytes, formatDateCompact } from "@/lib/format"
import type { AssetSummary, GalleryCardDisplay } from "@/lib/ipc-contract"
import { AssetCell } from "./AssetCell"
import { DEFAULT_CARD_DISPLAY, PRESETS, type CardDisplayPatch } from "./cardDisplay"

afterEach(() => vi.restoreAllMocks())

const CAPTURED = Date.UTC(2026, 7, 2, 12)
const MODIFIED = Date.UTC(2026, 9, 20, 12)

const video: AssetSummary = {
  assetId: 7,
  fileName: "python-basics-en.mp4",
  mediaType: "video",
  availability: "online",
  width: 1920,
  height: 1080,
  durationMs: 425_000,
  fileSize: 48_200_000,
  capturedAt: CAPTURED,
  modifiedAt: MODIFIED,
  favorite: false,
  rating: 0,
  thumbState: "pending",
  aiState: "none",
  title: null,
  revision: 1,
}

const display = (patch: CardDisplayPatch = {}): GalleryCardDisplay => ({
  ...DEFAULT_CARD_DISPLAY,
  ...patch,
  info: { ...DEFAULT_CARD_DISPLAY.info, ...patch.info },
})

interface Opts {
  asset?: Partial<AssetSummary>
  display?: GalleryCardDisplay
  cellWidth?: number
  selected?: boolean
  selecting?: boolean
}

function setup(o: Opts = {}) {
  const onSelect = vi.fn()
  const onOpen = vi.fn()
  const onToggleFavorite = vi.fn()
  const asset = { ...video, ...o.asset }
  const view = render(
    <ApiProvider api={createMockApi({ count: 1 })}>
      <div role="listbox" aria-label="목록" tabIndex={0}>
        <AssetCell
          asset={asset}
          selected={o.selected ?? false}
          display={o.display ?? display()}
          cellWidth={o.cellWidth ?? 200}
          selecting={o.selecting ?? false}
          onSelect={onSelect}
          onOpen={onOpen}
          onToggleFavorite={onToggleFavorite}
        />
      </div>
    </ApiProvider>
  )
  const card = screen.getByRole("option")
  return { user: userEvent.setup(), card, onSelect, onOpen, onToggleFavorite, view }
}

describe("AssetCell info area", () => {
  it("T-ui-cell-01 [Happy] detailed preset shows name, extension chip, resolution, size and date; compact only name and chip; off shows no text", () => {
    // Arrange + Act: 자세히
    const a = setup()
    // Assert
    expect(within(a.card).getByText("python-basics-en")).toBeInTheDocument()
    expect(within(a.card).getByText("MP4")).toBeInTheDocument()
    const meta = `1920×1080 · ${formatBytes(48_200_000)} · ${formatDateCompact(CAPTURED)}`
    expect(within(a.card).getByText(meta)).toBeInTheDocument()
    a.view.unmount()

    // Act: 간단
    const b = setup({ display: display({ info: PRESETS.compact }) })
    expect(within(b.card).getByText("python-basics-en")).toBeInTheDocument()
    expect(within(b.card).queryByText(/1920×1080/)).not.toBeInTheDocument()
    b.view.unmount()

    // Act: 썸네일만
    const c = setup({ display: display({ info: PRESETS.off }) })
    expect(within(c.card).queryByText("python-basics-en")).not.toBeInTheDocument()
    expect(within(c.card).queryByText("MP4")).not.toBeInTheDocument()
    expect(within(c.card).queryByText(/MB/)).not.toBeInTheDocument()
  })

  it("T-ui-cell-02 [Boundary] extension off shows the full file name; a single item off hides only that item", () => {
    // Arrange + Act
    const { card } = setup({ display: display({ info: { ext: false } }) })
    // Assert
    expect(within(card).getByText("python-basics-en.mp4")).toBeInTheDocument()
    expect(within(card).queryByText("MP4")).not.toBeInTheDocument()
    expect(within(card).getByText(/1920×1080/)).toBeInTheDocument()
  })

  it("T-ui-cell-03 [Happy] audio shows no resolution even if the asset carries dimensions", () => {
    // Arrange + Act
    const { card } = setup({ asset: { mediaType: "audio", fileName: "take3.wav", width: 640, height: 480 } })
    // Assert
    expect(within(card).queryByText(/640×480/)).not.toBeInTheDocument()
    expect(within(card).getByText(`${formatBytes(48_200_000)} · ${formatDateCompact(CAPTURED)}`)).toBeInTheDocument()
  })

  it("T-ui-cell-04 [Boundary] date basis: captured-first falls back to the modified date; modified uses the modified date", () => {
    // Arrange + Act + Assert: 촬영일 있음
    const only = { info: { resolution: false, size: false } }
    const a = setup({ display: display(only) })
    expect(within(a.card).getByText(formatDateCompact(CAPTURED))).toBeInTheDocument()
    a.view.unmount()
    // 촬영일 없음 → 수정일
    const b = setup({ asset: { capturedAt: null }, display: display(only) })
    expect(within(b.card).getByText(formatDateCompact(MODIFIED))).toBeInTheDocument()
    b.view.unmount()
    // 수정일 기준
    const c = setup({ display: display({ dateBy: "modified", ...only }) })
    expect(within(c.card).getByText(formatDateCompact(MODIFIED))).toBeInTheDocument()
    expect(within(c.card).queryByText(formatDateCompact(CAPTURED))).not.toBeInTheDocument()
  })

  it("T-ui-cell-05 [Boundary] under 140px the meta line and the extension chip are hidden, the name stays", () => {
    // Arrange + Act
    const { card } = setup({ cellWidth: 132 })
    // Assert: 확장자 칩이 없으니 파일명은 확장자 포함 전체
    expect(within(card).getByText("python-basics-en.mp4")).toBeInTheDocument()
    expect(within(card).queryByText("MP4")).not.toBeInTheDocument()
    expect(within(card).queryByText(/1920×1080/)).not.toBeInTheDocument()
  })
})

describe("AssetCell overlay", () => {
  it("T-ui-cell-06 [Happy] accessible name carries name, type, duration and status; images have no duration", () => {
    // Arrange + Act + Assert
    const a = setup({ asset: { availability: "offline" } })
    expect(a.card).toHaveAccessibleName("python-basics-en.mp4, 동영상, 7:05, 오프라인")
    a.view.unmount()
    const b = setup({ asset: { mediaType: "image", durationMs: null, fileName: "a.png" } })
    expect(b.card).toHaveAccessibleName("a.png, 이미지")
  })

  it.each([
    ["hover", true],
    ["always", true],
    ["hidden", false],
  ] as const)("T-ui-cell-07 [Business rule] check mode %s: check button present=%s; Ctrl-click on the card still selects", async (check, present) => {
    // Arrange
    const { user, card, onSelect } = setup({ display: display({ check }) })
    // Act
    await user.keyboard("{Control>}")
    await user.click(card)
    await user.keyboard("{/Control}")
    // Assert
    expect(screen.queryByRole("button", { name: "선택" }) !== null).toBe(present)
    expect(onSelect).toHaveBeenCalledExactlyOnceWith(7, { ctrl: true, shift: false })
  })

  it("T-ui-cell-08 [Business rule] check click goes through the ctrl path only and never opens the preview", async () => {
    // Arrange
    const { user, onSelect, onOpen } = setup()
    const check = screen.getByRole("button", { name: "선택" })
    // Act
    await user.click(check)
    await user.dblClick(check)
    // Assert: 클릭 1 + 더블클릭의 클릭 2 = 3번, 모두 ctrl 경로 (카드의 일반 클릭은 없다)
    expect(onSelect).toHaveBeenCalledTimes(3)
    for (const call of onSelect.mock.calls) expect(call).toEqual([7, { ctrl: true, shift: false }])
    expect(onOpen).not.toHaveBeenCalled()
  })

  it("T-ui-cell-09 [Business rule] star click toggles favorite only (no selection, no open) and overlay buttons stay out of the tab order", async () => {
    // Arrange
    const { user, onSelect, onOpen, onToggleFavorite } = setup()
    const star = screen.getByRole("button", { name: "즐겨찾기에 추가" })
    // Act
    await user.click(star)
    await user.dblClick(star)
    // Assert
    expect(onToggleFavorite).toHaveBeenCalledWith(7, true)
    expect(onSelect).not.toHaveBeenCalled()
    expect(onOpen).not.toHaveBeenCalled()
    expect(star).toHaveAttribute("tabindex", "-1")
    expect(screen.getByRole("button", { name: "선택" })).toHaveAttribute("tabindex", "-1")
    expect(screen.getByRole("button", { name: "큰 보기" })).toHaveAttribute("tabindex", "-1")
  })

  it("T-ui-cell-10 [Business rule] a favorite asset offers 'remove' (favorite=false); quick view opens the preview without selecting", async () => {
    // Arrange
    const { user, onSelect, onOpen, onToggleFavorite } = setup({ asset: { favorite: true } })
    // Act
    await user.click(screen.getByRole("button", { name: "즐겨찾기 해제" }))
    await user.click(screen.getByRole("button", { name: "큰 보기" }))
    await user.dblClick(screen.getByRole("button", { name: "큰 보기" }))
    // Assert: 더블클릭이 셀의 더블클릭(열기)으로 번지지 않아 클릭 수(1 + 2)만큼만 호출된다
    expect(onToggleFavorite).toHaveBeenCalledWith(7, false)
    expect(onOpen).toHaveBeenCalledTimes(3)
    expect(onOpen).toHaveBeenCalledWith(7)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it("T-ui-cell-11 [Business rule] turning favorite / duration / quick actions off removes only that element", () => {
    // Arrange + Act + Assert: 즐겨찾기 끔
    const a = setup({ display: display({ favorite: false }) })
    expect(screen.queryByRole("button", { name: "즐겨찾기에 추가" })).not.toBeInTheDocument()
    expect(screen.getByText("7:05")).toBeInTheDocument()
    a.view.unmount()
    // 재생 길이 끔 (큰 보기는 유지)
    const b = setup({ display: display({ duration: false }) })
    expect(screen.queryByText("7:05")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "큰 보기" })).toBeInTheDocument()
    b.view.unmount()
    // 빠른 동작 끔 (길이는 유지)
    setup({ display: display({ quickActions: false }) })
    expect(screen.queryByRole("button", { name: "큰 보기" })).not.toBeInTheDocument()
    expect(screen.getByText("7:05")).toBeInTheDocument()
  })

  it("T-ui-cell-12 [Business rule] offline / missing show the status in the meta line, the thumbnail chip (wide cards only) and the accessible name", () => {
    // Arrange + Act: 오프라인, 넓은 카드 → 메타 줄 + 칩 = 2곳
    const a = setup({ asset: { availability: "offline" } })
    expect(within(a.card).getAllByText("오프라인")).toHaveLength(2)
    expect(a.card).toHaveAccessibleName(expect.stringContaining("오프라인"))
    a.view.unmount()
    // 파일 없음, 160px 미만 → 칩은 아이콘만(제목으로 상태 노출), 메타 줄 글자 1곳
    const b = setup({ asset: { availability: "missing" }, cellWidth: 150 })
    expect(within(b.card).getAllByText("파일 없음")).toHaveLength(1)
    expect(b.card).toHaveAccessibleName(expect.stringContaining("파일 없음"))
    b.view.unmount()
    // 정상은 상태 글자가 없다
    const c = setup()
    expect(within(c.card).queryByText(/오프라인|파일 없음/)).not.toBeInTheDocument()
  })

  it("T-ui-cell-13 [Business rule] status stays visible when every info item is off (the thumbnail chip is not optional)", () => {
    // Arrange + Act
    const { card } = setup({ asset: { availability: "offline" }, display: display({ info: PRESETS.off }) })
    // Assert
    expect(within(card).getAllByText("오프라인")).toHaveLength(1)
  })

  it("T-ui-cell-14 [Happy] selected state is exposed via aria-selected and the check button label", () => {
    // Arrange + Act
    const { card } = setup({ selected: true })
    // Assert
    expect(card).toHaveAttribute("aria-selected", "true")
    expect(screen.getByRole("button", { name: "선택 해제" })).toHaveAttribute("aria-pressed", "true")
  })
})
