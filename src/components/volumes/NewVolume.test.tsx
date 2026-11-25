// Categories: Happy (T-ui-vol-01 02 05 07) / Business rule (03 04 remember) / Validation (06 outside) / External (08 add failure) / Boundary (09 hidden when empty)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { IgnoredVolume, PendingVolume } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

const USB: PendingVolume = { volumeKey: "vol-usb", label: "SanDisk USB", kind: "removable", mountPoint: "E:\\", totalBytes: 62_500_000_000 }
const BAK: PendingVolume = { volumeKey: "vol-bak", label: null, kind: "fixed", mountPoint: "F:\\", totalBytes: null }
const IGN: IgnoredVolume = { volumeKey: "vol-cam", label: "CAMERA", lastMountPoint: "G:\\", ignoredAt: Date.UTC(2026, 10, 10, 3) }

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 앱을 띄우고 이벤트 핸들러를 가로채 테스트가 직접 "백엔드 이벤트" 를 보낼 수 있게 한다. */
async function setup(opts: Parameters<typeof createMockApi>[0] = {}) {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 5, ...opts })
  let push: (v: PendingVolume[]) => void = () => undefined
  vi.spyOn(api, "onPendingVolumesChanged").mockImplementation(async (h) => {
    push = h
    return () => undefined
  })
  const dismiss = vi.spyOn(api, "dismissNewVolume")
  renderApp(api)
  await screen.findByRole("navigation", { name: "라이브러리 탐색" })
  await waitFor(() => expect(api.onPendingVolumesChanged).toHaveBeenCalled())
  return { user, api, dismiss, emit: (v: PendingVolume[]) => act(() => push(v)) }
}

const dialog = () => screen.findByRole("dialog", { name: "새 디스크가 연결되었습니다" })

describe("new volume dialog", () => {
  it("T-ui-vol-01 [Happy] opens from the initial pending list with one card per volume (label · mount, kind, size)", async () => {
    // Arrange + Act
    await setup({ volumes: { pending: [USB, BAK] } })
    const dlg = await dialog()
    // Assert
    const card = within(dlg).getByRole("region", { name: "SanDisk USB (E:\\)" })
    expect(within(card).getByText(/이동식 · 58\.2 GB/)).toBeInTheDocument()
    expect(within(dlg).getByRole("region", { name: "F:\\" })).toBeInTheDocument()
    expect(within(dlg).getByText("시스템 폴더(휴지통 등)는 자동으로 제외됩니다.")).toBeInTheDocument()
    expect(within(dlg).getByRole("checkbox", { name: "이 기기에서 선택한 사항 기억하기" })).toBeChecked()
  })

  it("T-ui-vol-02 [Happy] shows on the pending-changed event, card removed by a later event, closes when empty", async () => {
    // Arrange
    const { emit } = await setup()
    expect(screen.queryByRole("dialog", { name: "새 디스크가 연결되었습니다" })).not.toBeInTheDocument()
    // Act + Assert: 이벤트로 열림
    await emit([USB, BAK])
    const dlg = await dialog()
    expect(within(dlg).getAllByRole("region")).toHaveLength(2)
    // 한 장 제거
    await emit([BAK])
    await waitFor(() => expect(within(dlg).getAllByRole("region")).toHaveLength(1))
    expect(within(dlg).queryByRole("region", { name: "SanDisk USB (E:\\)" })).not.toBeInTheDocument()
    // 비면 닫힘
    await emit([])
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "새 디스크가 연결되었습니다" })).not.toBeInTheDocument())
  })

  it("T-ui-vol-03 [Business rule] 나중에 sends remember=true by default and remember=false after unchecking", async () => {
    // Arrange
    const { user, dismiss } = await setup({ volumes: { pending: [USB, BAK] } })
    const dlg = await dialog()
    // Act 1: 기본(기억함)
    await user.click(within(within(dlg).getByRole("region", { name: "SanDisk USB (E:\\)" })).getByRole("button", { name: "나중에" }))
    await waitFor(() => expect(dismiss).toHaveBeenCalledWith("vol-usb", true))
    // Act 2: 체크 해제
    await user.click(within(dlg).getByRole("checkbox", { name: "이 기기에서 선택한 사항 기억하기" }))
    await user.click(within(await within(dlg).findByRole("region", { name: "F:\\" })).getByRole("button", { name: "나중에" }))
    // Assert
    await waitFor(() => expect(dismiss).toHaveBeenCalledWith("vol-bak", false))
    expect(dismiss).toHaveBeenCalledTimes(2)
  })

  it("T-ui-vol-04 [Business rule] 닫기 dismisses every remaining card following the remember checkbox, then the dialog closes", async () => {
    // Arrange
    const { user, dismiss, emit } = await setup({ volumes: { pending: [USB, BAK] } })
    const dlg = await dialog()
    await user.click(within(dlg).getByRole("checkbox", { name: "이 기기에서 선택한 사항 기억하기" }))
    // Act: 푸터 닫기 (Radix 기본 X 버튼도 이름이 "닫기"/"Close" 일 수 있어 푸터 안에서 찾는다)
    const buttons = within(dlg).getAllByRole("button", { name: "닫기" })
    await user.click(buttons[0])
    // Assert
    await waitFor(() => expect(dismiss).toHaveBeenCalledTimes(2))
    expect(dismiss.mock.calls.map((c) => c.join(":")).sort()).toEqual(["vol-bak:false", "vol-usb:false"])
    // 백엔드가 대기 목록을 비우는 이벤트를 보내면 닫힌다
    await emit([])
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "새 디스크가 연결되었습니다" })).not.toBeInTheDocument())
  })

  it("T-ui-vol-04b [Business rule] 닫기 with the checkbox checked dismisses with remember=true", async () => {
    // Arrange
    const { user, dismiss } = await setup({ volumes: { pending: [USB] } })
    const dlg = await dialog()
    // Act
    await user.click(within(dlg).getAllByRole("button", { name: "닫기" })[0])
    // Assert
    await waitFor(() => expect(dismiss).toHaveBeenCalledWith("vol-usb", true))
  })

  it("T-ui-vol-04c [Business rule] Esc dismisses the remaining cards with remember=false even when the checkbox is checked", async () => {
    // Arrange
    const { user, dismiss } = await setup({ volumes: { pending: [USB, BAK] } })
    const dlg = await dialog()
    expect(within(dlg).getByRole("checkbox", { name: "이 기기에서 선택한 사항 기억하기" })).toBeChecked()
    // Act
    await user.keyboard("{Escape}")
    // Assert
    await waitFor(() => expect(dismiss).toHaveBeenCalledTimes(2))
    expect(dismiss.mock.calls.map((c) => c.join(":")).sort()).toEqual(["vol-bak:false", "vol-usb:false"])
  })

  it("T-ui-vol-05 [Happy] 추가 with 디스크 전체 registers the mount point with the chosen media types", async () => {
    // Arrange
    const { user, api } = await setup({ volumes: { pending: [USB] } })
    const add = vi.spyOn(api, "addFolder")
    const dlg = await dialog()
    await user.click(within(dlg).getByRole("checkbox", { name: "음성" }))
    // Act
    await user.click(within(dlg).getByRole("button", { name: "추가" }))
    // Assert
    await waitFor(() => expect(add).toHaveBeenCalledWith("E:\\", { image: true, video: true, audio: false }))
  })

  it("T-ui-vol-06 [Validation] a folder outside the disk is rejected inline (aria-invalid) and not registered; a folder inside is registered", async () => {
    // Arrange
    const { user, api } = await setup({ volumes: { pending: [USB] } })
    const add = vi.spyOn(api, "addFolder")
    const pick = vi.spyOn(api, "pickDirectory").mockResolvedValueOnce("D:\\Other").mockResolvedValueOnce("e:/media")
    const dlg = await dialog()
    await user.click(within(dlg).getByRole("radio", { name: "폴더 선택…" }))
    const addBtn = within(dlg).getByRole("button", { name: "추가" })
    expect(addBtn).toBeDisabled() // 아직 고른 폴더 없음
    // Act 1: 디스크 밖
    await user.click(within(dlg).getByRole("button", { name: "찾기" }))
    expect(pick).toHaveBeenCalledWith("E:\\")
    // Assert 1
    const input = within(dlg).getByRole("textbox", { name: "등록할 폴더" })
    const err = await within(dlg).findByRole("alert")
    expect(err).toHaveTextContent("이 디스크(E:\\) 안에 있어야 합니다")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input.getAttribute("aria-describedby")).toBe(err.id)
    expect(addBtn).toBeDisabled()
    expect(add).not.toHaveBeenCalled()
    // Act 2: 디스크 안 (대소문자·슬래시 무관)
    await user.click(within(dlg).getByRole("button", { name: "찾기" }))
    await waitFor(() => expect(addBtn).toBeEnabled())
    await user.click(addBtn)
    // Assert 2
    await waitFor(() => expect(add).toHaveBeenCalledWith("e:/media", expect.anything()))
    expect(pick).toHaveBeenCalledTimes(2)
  })

  it("T-ui-vol-07 [Business rule] no media type selected blocks 추가", async () => {
    // Arrange
    const { user, api } = await setup({ volumes: { pending: [USB] } })
    const add = vi.spyOn(api, "addFolder")
    const dlg = await dialog()
    // Act
    for (const name of ["동영상", "이미지", "음성"]) await user.click(within(dlg).getByRole("checkbox", { name }))
    // Assert
    expect(within(dlg).getByText("하나 이상 선택하세요.")).toBeInTheDocument()
    expect(within(dlg).getByRole("button", { name: "추가" })).toBeDisabled()
    expect(add).not.toHaveBeenCalled()
  })

  it("T-ui-vol-08 [External] add_folder failure is shown inline under the card, the card stays and 추가 can be retried", async () => {
    // Arrange
    const { user, api } = await setup({ volumes: { pending: [USB, BAK] } })
    const add = vi.spyOn(api, "addFolder").mockRejectedValueOnce({ code: "nested_folder", message: "overlap" })
    const dlg = await dialog()
    const card = within(dlg).getByRole("region", { name: "SanDisk USB (E:\\)" })
    // Act
    await user.click(within(card).getByRole("button", { name: "추가" }))
    // Assert
    expect(await within(card).findByRole("alert")).toHaveTextContent("MAM 데이터·백업 폴더와 겹치는 위치입니다")
    expect(within(dlg).getByRole("region", { name: "F:\\" })).toBeInTheDocument()
    expect(within(card).getByRole("button", { name: "추가" })).toBeEnabled()
    // 재시도 성공 시 에러 해제
    await user.click(within(card).getByRole("button", { name: "추가" }))
    await waitFor(() => expect(add).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(within(card).queryByRole("alert")).not.toBeInTheDocument())
  })
})

describe("ignored volumes in settings", () => {
  it("T-ui-vol-09 [Happy] lists ignored disks (name · last mount · date) and 다시 묻기 unignores then refetches", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 5, volumes: { ignored: [IGN], connected: [{ ...USB, volumeKey: "vol-cam", label: "CAMERA", mountPoint: "G:\\" }] } })
    const unignore = vi.spyOn(api, "unignoreVolume")
    renderApp(api)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    const settings = await openSettingsPage(user, "관리 폴더")
    // Assert: 목록
    const list = await within(settings).findByRole("list", { name: "기억하는 디스크" })
    expect(within(list).getByText("CAMERA")).toBeInTheDocument()
    expect(within(list).getByText("묻지 않음")).toBeInTheDocument()
    expect(within(list).getByText("G:\\")).toBeInTheDocument()
    // Act
    await user.click(within(list).getByRole("button", { name: "CAMERA 다시 묻기" }))
    // Assert: 호출 + 목록 사라짐 + 연결 중이므로 새 디스크 Dialog 가 열림 (mock 이벤트)
    await waitFor(() => expect(unignore).toHaveBeenCalledWith("vol-cam"))
    await waitFor(() => expect(within(settings).queryByRole("heading", { name: "기억하는 디스크" })).not.toBeInTheDocument())
    expect(await screen.findByRole("dialog", { name: "새 디스크가 연결되었습니다" })).toBeInTheDocument()
  })

  it("T-ui-vol-10 [Boundary] the section is hidden when nothing is ignored", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 5 })
    const list = vi.spyOn(api, "listKnownVolumes")
    renderApp(api)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    // Act
    const settings = await openSettingsPage(user, "관리 폴더")
    await waitFor(() => expect(list).toHaveBeenCalled())
    // Assert
    expect(within(settings).queryByText("기억하는 디스크")).not.toBeInTheDocument()
    expect(screen.queryByRole("dialog", { name: "새 디스크가 연결되었습니다" })).not.toBeInTheDocument()
  })
})
