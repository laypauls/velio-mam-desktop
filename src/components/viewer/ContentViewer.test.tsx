// Categories: Happy (T-ui-viewer-01..04, -06, -11) / Business rule (T-ui-viewer-05, -07, -09, -10, -12) / Boundary (T-ui-viewer-08, -13)
//             External failure (T-ui-viewer-14) / Validation (T-ui-viewer-15..17: keyboard scoping, Esc scoping)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { qk } from "@/hooks/keys"
import { createMockApi } from "@/lib/api/mock"
import type { AssetDetail } from "@/lib/ipc-contract"
import { resetAllStores, useSelectionStore, useViewStore } from "@/stores"
import { VIEWER_DEFAULT_WIDTH, VIEWER_MIN_WIDTH } from "@/stores/view"
import { stubMedia } from "@/test/media"
import { ContentViewer } from "./ContentViewer"

// 내장 플레이어(WebGL 캔버스·SharedBuffer)은 jsdom 에서 열 수 없다 — 어떤 엔진이 선택됐는지만 보는 스텁
vi.mock("./MpvPlayer", () => ({
  MpvPlayer: ({ name }: { name: string }) => <div role="group" aria-label={`mpv:${name}`} />,
}))

/** ResizeObserver 는 콜백을 부르지 않는 스텁(Radix Slider 가 필요로 한다) — 크기는 clientWidth/Height 에서 읽는다 */
function stubResizeObserver() {
  class NoopResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  vi.stubGlobal("ResizeObserver", NoopResizeObserver)
}

function stubBox(width: number, height: number) {
  vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(width)
  vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(height)
}

beforeEach(() => {
  resetAllStores()
  stubResizeObserver()
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 기본 상세를 덮어쓴 자산으로 보기 영역을 연다. `outside` 는 갤러리 역할의 포커스 가능 요소. */
async function openWith(patch: Partial<AssetDetail>, opts: { open?: boolean; vlcSetting?: boolean; vlcAvailable?: boolean; placement?: "left" | "right" } = {}) {
  const api = createMockApi({ count: 20 })
  const base = await api.getAssetDetail(1)
  const detail: AssetDetail = { ...base, availability: "online", ...patch }
  vi.spyOn(api, "getAssetDetail").mockImplementation((id) => Promise.resolve({ ...detail, assetId: id }))
  vi.spyOn(api, "mediaUrl").mockImplementation((id) => `media://${id}`)
  // 열 때 자동 재생 — 정지 상태 설정이면 영상은 포스터가 먼저 뜬다 (VideoPoster.test.tsx)
  await api.updateSettings({ mediaOpenAutoplay: true })
  if (opts.placement) await api.updateSettings({ viewerPlacement: opts.placement })
  if (opts.vlcSetting !== undefined) await api.updateSettings({ videoPlayerVlc: opts.vlcSetting })
  vi.spyOn(api, "playerAvailable").mockResolvedValue(opts.vlcAvailable ?? false)
  const thumbUrl = vi.spyOn(api, "thumbUrl").mockImplementation((id, size) => `thumb://${id}/${size}`)
  const openOriginal = vi.spyOn(api, "openOriginal").mockResolvedValue(undefined)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  if (opts.placement) queryClient.setQueryData(qk.settings, await api.getSettings()) // 설정 로드 전 기본(left)으로 첫 렌더되는 경합 방지
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <main>
          <button type="button">gallery</button>
          <input aria-label="search" />
        </main>
        <ContentViewer />
      </QueryClientProvider>
    </ApiProvider>
  )
  if (opts.open !== false) act(() => useViewStore.getState().openPreview(detail.assetId))
  return { detail, thumbUrl, openOriginal, api, queryClient }
}


describe("ContentViewer — pane", () => {
  it("T-ui-viewer-01 [Happy] opens only while previewId is set and shows type badge, name and resolution/fps/duration", async () => {
    // Arrange
    stubMedia()
    await openWith(
      { mediaType: "video", playable: true, fileName: "clip.mp4", width: 1920, height: 1080, fps: 29.97, durationMs: 83_000 },
      { open: false }
    )
    expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).not.toBeInTheDocument()
    // Act
    act(() => useViewStore.getState().openPreview(1))
    // Assert
    const pane = await screen.findByRole("complementary", { name: "콘텐츠 보기" })
    expect(await within(pane).findByRole("heading", { name: "clip.mp4" })).toBeInTheDocument()
    expect(within(pane).getByText("동영상")).toBeInTheDocument()
    expect(within(pane).getByText("1920×1080 · 29.97fps · 1:23")).toBeInTheDocument()
  })

  it("T-ui-viewer-02 [Happy] the close button clears previewId and removes the pane", async () => {
    // Arrange
    const user = userEvent.setup()
    await openWith({ mediaType: "image", playable: true, fileName: "a.jpg" })
    await screen.findByRole("heading", { name: "a.jpg" })
    // Act
    await user.click(screen.getByRole("button", { name: "보기 영역 닫기" }))
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).not.toBeInTheDocument()
  })

  it("T-ui-viewer-03 [Business rule] the viewer follows the active asset while open", async () => {
    // Arrange
    await openWith({ mediaType: "image", playable: true, fileName: "a.jpg" })
    await screen.findByRole("heading", { name: "a.jpg" })
    // Act
    act(() => useSelectionStore.getState().selectOnly(2))
    // Assert
    expect(useViewStore.getState().previewId).toBe(2)
    expect(await screen.findByRole("img", { name: "a.jpg" })).toHaveAttribute("src", "media://2")
  })

  it("T-ui-viewer-04 [Business rule] selecting an asset does not open the viewer while it is closed", async () => {
    // Arrange
    await openWith({ mediaType: "image", playable: true }, { open: false })
    // Act
    act(() => useSelectionStore.getState().selectOnly(2))
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-viewer-13 [Boundary] the resize handle changes the width by keyboard and never goes below the minimum", async () => {
    // Arrange
    const user = userEvent.setup()
    await openWith({ mediaType: "image", playable: true, }, { placement: "right" })
    const handle = await screen.findByRole("separator", { name: "보기 영역 폭 조절" })
    expect(useViewStore.getState().viewerWidth).toBe(VIEWER_DEFAULT_WIDTH)
    // Act
    handle.focus()
    await user.keyboard("{ArrowLeft}")
    const widened = useViewStore.getState().viewerWidth
    for (let i = 0; i < 40; i++) await user.keyboard("{ArrowRight}")
    // Assert
    expect(widened).toBeGreaterThan(VIEWER_DEFAULT_WIDTH)
    expect(useViewStore.getState().viewerWidth).toBe(VIEWER_MIN_WIDTH)
    act(() => useViewStore.getState().setViewerWidth(10))
    expect(useViewStore.getState().viewerWidth).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-viewer-18 [Business rule] default placement is left: ArrowRight widens the viewer, ArrowLeft narrows it", async () => {
    // Arrange (설정을 고르지 않은 기본 상태)
    const user = userEvent.setup()
    await openWith({ mediaType: "image", playable: true })
    const handle = await screen.findByRole("separator", { name: "보기 영역 폭 조절" })
    // Act
    handle.focus()
    await user.keyboard("{ArrowRight}")
    const widened = useViewStore.getState().viewerWidth
    await user.keyboard("{ArrowLeft}{ArrowLeft}")
    // Assert
    expect(widened).toBeGreaterThan(VIEWER_DEFAULT_WIDTH)
    expect(useViewStore.getState().viewerWidth).toBeLessThan(widened)
  })
})

describe("ContentViewer — video / audio", () => {
  it("T-ui-viewer-05 [Business rule] a playable video uses the media URL without native controls and play/pause toggles", async () => {
    // Arrange
    const media = stubMedia()
    const user = userEvent.setup()
    const { detail } = await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    await user.click(screen.getByRole("button", { name: "재생" }))
    // Assert
    expect(video).toHaveAttribute("src", `media://${detail.assetId}`)
    expect(video).not.toHaveAttribute("controls")
    expect(media.play).toHaveBeenCalledTimes(1)
    await user.click(await screen.findByRole("button", { name: "일시정지" }))
    expect(media.pause).toHaveBeenCalledTimes(1)
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "외부 프로그램으로 열기" })).not.toBeInTheDocument()
  })

  it("T-ui-viewer-06 [Happy] changing the playback rate applies it to the element", async () => {
    // Arrange (Radix Select 가 jsdom 에 없는 API 를 쓴다)
    Element.prototype.hasPointerCapture ??= () => false
    Element.prototype.scrollIntoView ??= () => undefined
    stubMedia()
    const user = userEvent.setup()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    await user.click(screen.getByRole("combobox", { name: "재생 속도" }))
    await user.click(await screen.findByRole("option", { name: "1.5×" }))
    // Assert
    expect(video.playbackRate).toBe(1.5)
    expect(screen.getByRole("combobox", { name: "재생 속도" })).toHaveTextContent("1.5×")
  })

  it("T-ui-viewer-07 [Business rule] frame step pauses and moves by 1/fps from the asset fps", async () => {
    // Arrange
    const media = stubMedia()
    const user = userEvent.setup()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4", fps: 25 })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 10
    // Act
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    const forward = video.currentTime
    await user.click(screen.getByRole("button", { name: "뒤로 (정지 시 1프레임)" }))
    // Assert
    expect(forward).toBeCloseTo(10.04, 6)
    expect(video.currentTime).toBeCloseTo(10, 6)
    expect(media.pause).toHaveBeenCalledTimes(2)
  })

  it("T-ui-viewer-08 [Boundary] frame step falls back to 30 fps when the asset has no fps", async () => {
    // Arrange
    stubMedia()
    const user = userEvent.setup()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4", fps: null })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 1
    // Act
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    // Assert
    expect(video.currentTime).toBeCloseTo(1 + 1 / 30, 6)
  })

  it("T-ui-viewer-09 [Business rule] a seek request from the AI transcript moves the player and a later request moves it again", async () => {
    // Arrange
    stubMedia()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" }, { open: false })
    // Act
    act(() => useViewStore.getState().seekPreview(1, 14_000))
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Assert
    expect(video.currentTime).toBe(14)
    act(() => useViewStore.getState().seekPreview(1, 30_000))
    expect(video.currentTime).toBe(30)
  })

  it("T-ui-viewer-10 [Business rule] audio renders <audio> with the controls but no frame-step buttons", async () => {
    // Arrange
    stubMedia()
    const { detail } = await openWith({ mediaType: "audio", playable: true, fileName: "song.mp3" })
    // Act
    const audio = await screen.findByLabelText<HTMLAudioElement>("song.mp3", { selector: "audio" })
    // Assert
    expect(audio).toHaveAttribute("src", `media://${detail.assetId}`)
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "앞으로 (정지 시 1프레임)" })).not.toBeInTheDocument()
  })

  it("T-ui-viewer-11 [Business rule] mute toggles the element and the label", async () => {
    // Arrange
    stubMedia()
    const user = userEvent.setup()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    await user.click(screen.getByRole("button", { name: "음소거" }))
    // Assert
    expect(video.muted).toBe(true)
    expect(screen.getByRole("button", { name: "음소거 해제" })).toHaveAttribute("aria-pressed", "true")
  })
})

describe("ContentViewer — unavailable states", () => {
  it("T-ui-viewer-12 [Business rule] a non-playable video falls back to the external-open notice when the built-in player is unavailable (mock)", async () => {
    // Arrange
    const user = userEvent.setup()
    const { detail, openOriginal } = await openWith({ mediaType: "video", playable: false, fileName: "clip.mxf" })
    // Act
    expect(await screen.findByText(/내장 플레이어를 사용할 수 없어/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "외부 프로그램으로 열기" }))
    // Assert
    expect(openOriginal).toHaveBeenCalledWith(detail.assetId)
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-viewer-14 [External failure] an offline video shows the unavailable notice without an open action", async () => {
    // Arrange + Act
    await openWith({ mediaType: "video", playable: true, availability: "offline", fileName: "nas.mp4" })
    // Assert
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "외부 프로그램으로 열기" })).not.toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-viewer-18 [External failure] a failed detail load shows an alert instead of the player", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    vi.spyOn(api, "getAssetDetail").mockRejectedValue({ code: "io", message: "boom" })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <ApiProvider api={api}>
        <QueryClientProvider client={queryClient}>
          <ContentViewer />
        </QueryClientProvider>
      </ApiProvider>
    )
    // Act
    act(() => useViewStore.getState().openPreview(1))
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("불러오지 못했습니다")
    expect(document.querySelector("video")).toBeNull()
  })
})

describe("ContentViewer — keyboard scoping", () => {
  it("T-ui-viewer-15 [Validation] Space, arrows, comma/period and M act on the open player from any focus; not from an input", async () => {
    // Arrange
    const media = stubMedia()
    const user = userEvent.setup()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4", fps: 25 })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    // Act: 입력창 포커스에서는 무시
    screen.getByRole("textbox", { name: "search" }).focus()
    await user.keyboard(" {ArrowRight}.m")
    expect(media.play).not.toHaveBeenCalled()
    expect(video.currentTime).toBe(20)
    expect(video.muted).toBe(false)
    // Act: 갤러리 쪽 포커스 — 플레이어 단축키가 우선
    screen.getByRole("button", { name: "gallery" }).focus()
    await user.keyboard(" ")
    await user.keyboard("{ArrowRight}")
    const afterRight = video.currentTime
    await user.keyboard("{ArrowLeft}{ArrowLeft}")
    const afterLeft = video.currentTime
    media.state.paused = true // 정지 중에는 1프레임 (재생 중이면 초 단위 — T-ui-step-03)
    video.currentTime = 20
    await user.keyboard(".")
    const afterStep = video.currentTime
    await user.keyboard("m")
    // Assert
    expect(media.play).toHaveBeenCalledTimes(1)
    expect(afterRight).toBe(25)
    expect(afterLeft).toBe(15)
    expect(afterStep).toBeCloseTo(20.04, 6)
    expect(video.muted).toBe(true)
  })

  it("T-ui-viewer-16 [Validation] Escape closes the viewer, but not from an input and not while a menu layer is open", async () => {
    // Arrange
    const user = userEvent.setup()
    await openWith({ mediaType: "image", playable: true })
    await screen.findByRole("heading")
    // Act + Assert: 입력창에서는 무시
    screen.getByRole("textbox", { name: "search" }).focus()
    await user.keyboard("{Escape}")
    expect(useViewStore.getState().previewId).toBe(1)
    // 열린 레이어(대화상자 등)가 있으면 무시
    const layer = document.createElement("div")
    layer.setAttribute("role", "dialog")
    document.body.appendChild(layer)
    screen.getByRole("button", { name: "gallery" }).focus()
    await user.keyboard("{Escape}")
    expect(useViewStore.getState().previewId).toBe(1)
    layer.remove()
    // 그 외에는 닫힘
    await user.keyboard("{Escape}")
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-viewer-17 [Validation] an Escape already handled (defaultPrevented) by another handler does not close the viewer", async () => {
    // Arrange
    await openWith({ mediaType: "image", playable: true })
    await screen.findByRole("heading")
    const handled = (e: KeyboardEvent) => e.preventDefault()
    document.body.addEventListener("keydown", handled)
    // Act
    fireEvent.keyDown(document.body, { key: "Escape" })
    document.body.removeEventListener("keydown", handled)
    // Assert
    expect(useViewStore.getState().previewId).toBe(1)
  })
})

describe("ContentViewer — image", () => {
  async function openImage(patch: Partial<AssetDetail> = {}) {
    stubBox(500, 400)
    const ctx = await openWith({ mediaType: "image", playable: true, fileName: "photo.jpg", ...patch })
    const img = await screen.findByRole("img", { name: "photo.jpg" })
    Object.defineProperty(img, "naturalWidth", { value: 1000, configurable: true })
    Object.defineProperty(img, "naturalHeight", { value: 500, configurable: true })
    fireEvent.load(img)
    return { ...ctx, img }
  }

  it("T-ui-viewer-30 [Happy] fits to the stage by default, 1:1 and fit buttons switch the scale", async () => {
    // Arrange
    const user = userEvent.setup()
    const { img } = await openImage()
    // Assert: 1000x500 이미지를 500x400 영역에 맞춤
    expect(img).toHaveAttribute("data-scale", "0.5")
    expect(screen.getByRole("button", { name: "화면에 맞춤" })).toHaveAttribute("aria-pressed", "true")
    expect(screen.getByRole("status", { name: "배율 50%" })).toBeInTheDocument()
    // Act
    await user.click(screen.getByRole("button", { name: "원본 크기 (1:1)" }))
    // Assert
    expect(img).toHaveAttribute("data-scale", "1")
    await user.click(screen.getByRole("button", { name: "화면에 맞춤" }))
    expect(img).toHaveAttribute("data-scale", "0.5")
  })

  it("T-ui-viewer-31 [Happy] zoom buttons scale around the fitted size and stay within limits", async () => {
    // Arrange
    const user = userEvent.setup()
    const { img } = await openImage()
    // Act
    await user.click(screen.getByRole("button", { name: "확대" }))
    // Assert
    expect(Number(img.getAttribute("data-scale"))).toBeCloseTo(0.625, 6)
    expect(screen.getByRole("button", { name: "화면에 맞춤" })).toHaveAttribute("aria-pressed", "false")
    for (let i = 0; i < 40; i++) await user.click(screen.getByRole("button", { name: "확대" }))
    expect(Number(img.getAttribute("data-scale"))).toBe(16)
    for (let i = 0; i < 80; i++) await user.click(screen.getByRole("button", { name: "축소" }))
    expect(Number(img.getAttribute("data-scale"))).toBeCloseTo(0.05, 6)
  })

  it("T-ui-viewer-32 [Happy] rotate turns the display by 90 degrees (fit recomputed) without changing the source", async () => {
    // Arrange
    const user = userEvent.setup()
    const { img, detail } = await openImage()
    // Act
    await user.click(screen.getByRole("button", { name: "90° 회전" }))
    // Assert: 회전 후 500x1000 → 높이 400 에 맞춰 0.4
    expect(img).toHaveAttribute("data-rotation", "90")
    expect(img).toHaveAttribute("data-scale", "0.4")
    expect(img).toHaveAttribute("src", `media://${detail.assetId}`)
    for (let i = 0; i < 3; i++) await user.click(screen.getByRole("button", { name: "90° 회전" }))
    expect(img).toHaveAttribute("data-rotation", "0")
  })

  it("T-ui-viewer-33 [Happy] wheel zoom keeps the point under the cursor and dragging pans only when zoomed", async () => {
    // Arrange
    const { img } = await openImage()
    const stage = screen.getByTestId("image-stage")
    const translate = () => /translate\((-?[\d.e-]+)px, (-?[\d.e-]+)px\)/.exec(img.style.transform)?.slice(1).map(Number) ?? []
    // Act: 맞춤 상태의 드래그는 무시
    fireEvent.pointerDown(stage, { button: 0, clientX: 10, clientY: 10, pointerId: 1 })
    fireEvent.pointerMove(stage, { clientX: 60, clientY: 60, pointerId: 1 })
    fireEvent.pointerUp(stage, { pointerId: 1 })
    expect(translate()).toEqual([0, 0])
    // Act: 휠 확대 — 영역 좌표가 0 이므로 cursor = (100, 50)
    fireEvent.wheel(stage, { deltaY: -100, clientX: 100, clientY: 50 })
    // Assert: tx = px - (px - 0) * ratio, ratio = 1.1
    const [tx, ty] = translate()
    expect(Number(img.getAttribute("data-scale"))).toBeCloseTo(0.55, 6)
    expect(tx).toBeCloseTo(-10, 6)
    expect(ty).toBeCloseTo(-5, 6)
    // Act: 확대 상태의 드래그는 이동
    fireEvent.pointerDown(stage, { button: 0, clientX: 10, clientY: 10, pointerId: 1 })
    fireEvent.pointerMove(stage, { clientX: 40, clientY: 25, pointerId: 1 })
    fireEvent.pointerUp(stage, { pointerId: 1 })
    expect(translate()[0]).toBeCloseTo(tx + 30, 6)
    expect(translate()[1]).toBeCloseTo(ty + 15, 6)
  })

  it("T-ui-viewer-34 [Business rule] a non-playable image shows the 1024 thumbnail instead of the original", async () => {
    // Arrange + Act
    stubBox(500, 400)
    const { detail, thumbUrl } = await openWith({ mediaType: "image", playable: false, fileName: "raw_001.heic" })
    // Assert
    const img = await screen.findByRole("img", { name: "raw_001.heic" })
    expect(img).toHaveAttribute("src", `thumb://${detail.assetId}/1024`)
    expect(thumbUrl).toHaveBeenCalledWith(detail.assetId, 1024)
  })

  it("T-ui-viewer-35 [External failure] an image that fails to load shows an alert", async () => {
    // Arrange
    stubBox(500, 400)
    await openWith({ mediaType: "image", playable: true, fileName: "broken.jpg" })
    const img = await screen.findByRole("img", { name: "broken.jpg" })
    // Act
    fireEvent.error(img)
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("이미지를 표시하지 못했습니다")
  })
})

describe("ContentViewer — video engine setting (videoPlayerVlc)", () => {
  // Categories: Business rule (routing matrix) / Boundary (built-in player unavailable) / External failure (offline) / Concurrency (setting change while open)
  const vlcPlayer = (name: string) => screen.findByRole("group", { name: `mpv:${name}` })
  const htmlVideo = (name: string) => screen.findByLabelText(name, { selector: "video" })

  it("T-ui-vlcset-01 [Business rule] ON + playable video + built-in player available uses the built-in player, not <video>", async () => {
    stubMedia()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" }, { vlcSetting: true, vlcAvailable: true })
    expect(await vlcPlayer("clip.mp4")).toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-vlcset-02 [Business rule] ON + non-playable video + built-in player available uses the built-in player", async () => {
    await openWith({ mediaType: "video", playable: false, fileName: "clip.mxf" }, { vlcSetting: true, vlcAvailable: true })
    expect(await vlcPlayer("clip.mxf")).toBeInTheDocument()
  })

  it("T-ui-vlcset-03 [Boundary] ON + playable video + built-in player unavailable falls back to <video>", async () => {
    stubMedia()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" }, { vlcSetting: true, vlcAvailable: false })
    expect(await htmlVideo("clip.mp4")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "외부 프로그램으로 열기" })).not.toBeInTheDocument()
  })

  it("T-ui-vlcset-04 [Boundary] ON + non-playable video + built-in player unavailable shows the player-unavailable notice", async () => {
    await openWith({ mediaType: "video", playable: false, fileName: "clip.mxf" }, { vlcSetting: true, vlcAvailable: false })
    expect(await screen.findByText(/내장 플레이어를 사용할 수 없어/)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-vlcset-05 [Business rule] OFF + playable video keeps <video> even when built-in player is available", async () => {
    stubMedia()
    await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" }, { vlcSetting: false, vlcAvailable: true })
    expect(await htmlVideo("clip.mp4")).toBeInTheDocument()
    expect(screen.queryByRole("group", { name: "mpv:clip.mp4" })).not.toBeInTheDocument()
  })

  it("T-ui-vlcset-06 [Business rule] OFF + non-playable video uses the built-in player when available", async () => {
    await openWith({ mediaType: "video", playable: false, fileName: "clip.mxf" }, { vlcSetting: false, vlcAvailable: true })
    expect(await vlcPlayer("clip.mxf")).toBeInTheDocument()
  })

  it("T-ui-vlcset-07 [Boundary] OFF + non-playable video + built-in player unavailable shows the player-unavailable notice", async () => {
    await openWith({ mediaType: "video", playable: false, fileName: "clip.mxf" }, { vlcSetting: false, vlcAvailable: false })
    expect(await screen.findByText(/내장 플레이어를 사용할 수 없어/)).toBeInTheDocument()
  })

  it("T-ui-vlcset-08 [External failure] an offline video shows the offline notice with the setting ON and no player", async () => {
    await openWith({ mediaType: "video", playable: true, availability: "offline", fileName: "nas.mp4" }, { vlcSetting: true, vlcAvailable: true })
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    expect(screen.queryByRole("group", { name: "mpv:nas.mp4" })).not.toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-vlcset-09 [Business rule] audio never routes to the built-in player with the setting ON", async () => {
    stubMedia()
    await openWith({ mediaType: "audio", playable: true, fileName: "song.mp3" }, { vlcSetting: true, vlcAvailable: true })
    expect(await screen.findByLabelText("song.mp3", { selector: "audio" })).toBeInTheDocument()
    expect(screen.queryByRole("group", { name: "mpv:song.mp3" })).not.toBeInTheDocument()
  })

  it("T-ui-vlcset-10 [Business rule] non-playable audio keeps the external-open notice with the setting ON", async () => {
    await openWith({ mediaType: "audio", playable: false, fileName: "song.ape" }, { vlcSetting: true, vlcAvailable: true })
    expect(await screen.findByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
    expect(screen.queryByRole("group", { name: "mpv:song.ape" })).not.toBeInTheDocument()
  })

  it("T-ui-vlcset-11 [Concurrency] changing the setting while a video is open keeps its engine; the next opened video uses the new value", async () => {
    // Arrange
    stubMedia()
    const { queryClient, api } = await openWith({ mediaType: "video", playable: true, fileName: "clip.mp4" }, { vlcSetting: true, vlcAvailable: true })
    expect(await vlcPlayer("clip.mp4")).toBeInTheDocument()
    // Act: 재생 중 OFF
    const next = await api.updateSettings({ videoPlayerVlc: false })
    act(() => queryClient.setQueryData(qk.settings, next))
    // Assert: 현재 영상은 그대로
    expect(screen.getByRole("group", { name: "mpv:clip.mp4" })).toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
    // Act: 다음 영상 열기
    act(() => useSelectionStore.getState().selectOnly(2))
    // Assert: 새 설정(OFF) 적용 — playable 이므로 <video>
    expect(await htmlVideo("clip.mp4")).toBeInTheDocument()
    expect(screen.queryByRole("group", { name: "mpv:clip.mp4" })).not.toBeInTheDocument()
  })
})
