// Categories: Happy (T-ui-eject-01 02 05) / Business rule (03 subtitle, 06 viewer close only on that disk, 08 tooltip) / Boundary (04 network hidden, 07 viewer on another disk) / External (09 error toast)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { useViewStore } from "@/stores"

vi.mock("sonner", async (orig) => ({ ...(await orig<typeof import("sonner")>()), toast: { error: vi.fn(), success: vi.fn() } }))

const ALL = { image: true, video: true, audio: true }

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.mocked(toast.success).mockClear()
  vi.mocked(toast.error).mockClear()
})

/** E: 에 관리 폴더 `extra`+1 개를 추가한 뒤 앱을 띄운다 (기본 폴더: D:\Photos, D:\Videos, \\NAS\Archive) */
async function setup(extraOnE = 0, opts: { unplugged?: boolean } = {}) {
  stubLayout()
  const api = createMockApi({ count: 5 })
  await api.addFolder("E:\\data", ALL)
  for (let i = 0; i < extraOnE; i++) await api.addFolder(`E:\\more${i}`, ALL)
  if (opts.unplugged) {
    // 이미 케이블이 빠진 디스크 — 오프라인이지만 꺼내기 준비 상태는 아니다
    const real = api.listFolders.bind(api)
    vi.spyOn(api, "listFolders").mockImplementation(async () =>
      (await real()).map((f) => (f.path.startsWith("E:") ? { ...f, availability: "offline" as const } : f)),
    )
  }
  renderApp(api)
  await screen.findByRole("navigation", { name: "라이브러리 탐색" })
  await screen.findByRole("button", { name: "E:\\data" })
  return { api, user: userEvent.setup() }
}

const openMenu = (name: string) => fireEvent.contextMenu(screen.getByRole("button", { name }))

describe("sidebar folder eject menu", () => {
  it("T-ui-eject-01 [Happy] right-click on a local folder offers only 디스크 꺼내기 준비 (E:) and calls prepare for that folder", async () => {
    // Arrange
    const { api, user } = await setup()
    const prepare = vi.spyOn(api, "prepareVolumeEject")
    // Act
    openMenu("E:\\data")
    const items = await screen.findAllByRole("menuitem")
    // Assert
    expect(items).toHaveLength(1)
    expect(items[0]).toHaveTextContent("디스크 꺼내기 준비 (E:)")
    expect(screen.queryByText(/이 디스크의 관리 폴더/)).not.toBeInTheDocument()
    await user.click(items[0])
    await waitFor(() => expect(prepare).toHaveBeenCalledWith(4))
  })

  it("T-ui-eject-02 [Business rule] after prepare the toast names the mount point, folders go offline (tooltip 꺼낼 준비됨) and the menu flips to 감시 다시 시작", async () => {
    // Arrange
    const { api, user } = await setup()
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    // Assert
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("E:\\ 를 꺼낼 준비가 됐습니다. Windows 에서 안전하게 꺼내세요."))
    expect(await screen.findByTitle("꺼낼 준비됨")).toBeInTheDocument()
    expect(screen.getByRole("img", { name: "꺼낼 준비됨" })).toBeInTheDocument()
    expect((await api.listFolders()).find((f) => f.path === "E:\\data")).toMatchObject({ ejectPrepared: true, availability: "offline" })
    openMenu("E:\\data")
    expect(await screen.findByRole("menuitem")).toHaveTextContent("감시 다시 시작 (E:)")
  })

  it("T-ui-eject-03 [Business rule] 2+ managed folders on the same disk (hidden ones included) show the subtitle and all of them are prepared", async () => {
    // Arrange
    const { api, user } = await setup(1)
    await api.setFolderHidden(5, true)
    // Act
    openMenu("E:\\data")
    const item = await screen.findByRole("menuitem")
    // Assert
    expect(item).toHaveTextContent("이 디스크의 관리 폴더 2개 모두")
    await user.click(item)
    await waitFor(() => expect(toast.success).toHaveBeenCalled())
    const folders = await api.listFolders()
    expect(folders.filter((f) => f.path.startsWith("E:")).map((f) => f.ejectPrepared)).toEqual([true, true])
    expect(folders.filter((f) => f.path.startsWith("D:")).map((f) => f.ejectPrepared)).toEqual([false, false])
  })

  it("T-ui-eject-04 [Boundary] a network folder has no custom menu", async () => {
    // Arrange
    await setup()
    // Act
    openMenu("\\\\NAS\\Archive")
    // Assert
    await new Promise((r) => setTimeout(r, 50))
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument()
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()
  })

  it("T-ui-eject-10 [Boundary] an already-offline folder that is not ejectPrepared has no menu and never calls prepare", async () => {
    // Arrange
    const { api } = await setup(0, { unplugged: true })
    const prepare = vi.spyOn(api, "prepareVolumeEject")
    // Act
    openMenu("E:\\data")
    // Assert
    await new Promise((r) => setTimeout(r, 50))
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument()
    expect(prepare).not.toHaveBeenCalled()
  })

  it("T-ui-eject-11 [Business rule] an online internal-disk folder (ejectable false) has no menu — the gate is the backend flag, not the drive letter", async () => {
    // Arrange
    const { api } = await setup()
    const prepare = vi.spyOn(api, "prepareVolumeEject")
    expect((await api.listFolders()).find((f) => f.path === "D:\\Photos")).toMatchObject({ ejectable: false, availability: "online" })
    // Act
    openMenu("D:\\Photos")
    // Assert
    await new Promise((r) => setTimeout(r, 50))
    expect(screen.queryByRole("menu")).not.toBeInTheDocument()
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument()
    expect(prepare).not.toHaveBeenCalled()
  })

  it("T-ui-eject-05 [Happy] 감시 다시 시작 calls resume for that folder, toasts and brings the disk back online", async () => {
    // Arrange: 먼저 꺼내기 준비 상태로 만든다
    const { api, user } = await setup()
    const resume = vi.spyOn(api, "resumeVolumeWatch")
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    await screen.findByTitle("꺼낼 준비됨")
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem", { name: /감시 다시 시작 \(E:\)/ }))
    // Assert
    await waitFor(() => expect(resume).toHaveBeenCalledWith(4))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("감시를 다시 시작했습니다."))
    await waitFor(() => expect(screen.queryByTitle("꺼낼 준비됨")).not.toBeInTheDocument())
    expect((await api.listFolders()).find((f) => f.path === "E:\\data")).toMatchObject({ ejectPrepared: false, availability: "online" })
  })

  it("T-ui-eject-09 [External] a backend error shows the mapped error toast and leaves the folder unchanged", async () => {
    // Arrange
    const { api, user } = await setup()
    vi.spyOn(api, "prepareVolumeEject").mockRejectedValueOnce({ code: "offline", message: "x" })
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("저장 장치가 연결되어 있지 않습니다."))
    expect(toast.success).not.toHaveBeenCalled()
    expect(screen.queryByTitle("꺼낼 준비됨")).not.toBeInTheDocument()
  })
})

describe("prepare busy", () => {
  it("T-ui-eject-12 [External] busy from prepare shows the eject-specific message, not the shared folder-removal text", async () => {
    // Arrange
    const { api, user } = await setup()
    vi.spyOn(api, "prepareVolumeEject").mockRejectedValueOnce({ code: "busy", message: "x" })
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("디스크를 사용 중인 작업을 멈추지 못했습니다. 잠시 후 다시 시도하세요."))
    expect(screen.queryByTitle("꺼낼 준비됨")).not.toBeInTheDocument()
  })
})

describe("prepare closes the viewer only for that disk", () => {
  async function openViewerAt(path: string) {
    const { api, user } = await setup()
    const base = await api.getAssetDetail(1)
    vi.spyOn(api, "getAssetDetail").mockImplementation((id) => Promise.resolve({ ...base, assetId: id, mediaType: "image", availability: "online", path }))
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByRole("complementary", { name: "콘텐츠 보기" })
    await waitFor(() => expect(api.getAssetDetail).toHaveBeenCalled())
    return { user }
  }

  it("T-ui-eject-06 [Business rule] an open asset under the mount point (case-insensitive) closes the player panel", async () => {
    // Arrange
    const { user } = await openViewerAt("e:\\DATA\\clip.mp4")
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    // Assert
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).not.toBeInTheDocument())
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-eject-07 [Boundary] an open asset on another disk keeps the panel open", async () => {
    // Arrange
    const { user } = await openViewerAt("D:\\Photos\\a.jpg")
    // Act
    openMenu("E:\\data")
    await user.click(await screen.findByRole("menuitem"))
    // Assert
    await waitFor(() => expect(toast.success).toHaveBeenCalled())
    expect(screen.getByRole("complementary", { name: "콘텐츠 보기" })).toBeInTheDocument()
    expect(useViewStore.getState().previewId).toBe(1)
  })
})

describe("offline indicator", () => {
  it("T-ui-eject-08 [Business rule] only prepared folders get the 꺼낼 준비됨 tooltip — a plain offline folder keeps 오프라인", async () => {
    // Arrange + Act
    await setup()
    // Assert
    expect(screen.queryByTitle("꺼낼 준비됨")).not.toBeInTheDocument()
    const nas = screen.getByRole("button", { name: "\\\\NAS\\Archive" }).closest("div") as HTMLElement
    expect(within(nas).getByRole("img", { name: "오프라인" })).toBeInTheDocument()
  })
})
