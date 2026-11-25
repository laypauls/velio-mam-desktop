// Categories: Happy (T-ui-topbar-08 items + same actions, 10 card display, 11 settings) / Business rule (T-ui-topbar-09 semantic + list-mode disabled, 12 update badge/action) / Boundary (T-ui-topbar-13 360px markup contract)
// jsdom 은 컨테이너 쿼리를 계산하지 않는다 — 360px 단계는 "어떤 요소가 @max-[360px] 클래스를 갖는가" 라는 마크업 계약으로, 메뉴 동작은 접근성 역할로 검증한다.
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useProtectStore, useSearchStore, useViewStore } from "@/stores"
import { useUpdateStore } from "@/stores/update"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  useUpdateStore.setState({ available: null, dialogOpen: false })
})

async function setup() {
  stubLayout()
  stubMedia()
  const api = createMockApi({ count: 20 })
  await useStubbedDetail(api)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  return { user }
}

const topBar = (): HTMLElement => document.querySelector("[data-viewer-sibling] > header") as HTMLElement
const moreButton = () => within(topBar()).getByRole("button", { name: /^더보기/ })

async function openMore(user: ReturnType<typeof userEvent.setup>) {
  await user.click(moreButton())
  return screen.findByRole("menu")
}

/** 하위 메뉴 열기 — Radix 는 ArrowRight 로 연다 (jsdom 에는 pointer hover 이동이 신뢰할 수 없다) */
async function openSub(user: ReturnType<typeof userEvent.setup>, menu: HTMLElement, name: string) {
  const trigger = within(menu).getByRole("menuitem", { name })
  trigger.focus()
  await user.keyboard("{ArrowRight}")
  return trigger
}

const UPDATE = { available: true, currentVersion: "1.0.0", version: "1.2.0", notes: null, date: null }

describe("TopBar more (⋯) menu", () => {
  it("T-ui-topbar-08 [Happy] the menu lists sort/direction/view mode/card display/settings and sort + direction + view mode act like the wide controls", async () => {
    // Arrange
    const { user } = await setup()
    const dir0 = useSearchStore.getState().sort.direction
    // Act
    const menu = await openMore(user)
    // Assert: 항목
    for (const name of ["정렬 기준", "보기 방식", "카드 표시", "설정"]) expect(within(menu).getByRole("menuitem", { name })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitem", { name: /^정렬 방향/ })).toBeEnabled()
    // Act: 정렬 기준 → 제목
    await openSub(user, menu, "정렬 기준")
    await user.click(await screen.findByRole("menuitemradio", { name: "제목" }))
    // Assert
    expect(useSearchStore.getState().sort.field).toBe("name")
    // Act: 정렬 방향 토글
    await openMore(user)
    await user.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: /^정렬 방향/ }))
    // Assert
    expect(useSearchStore.getState().sort.direction).not.toBe(dir0)
    // Act: 보기 방식 → 목록
    const menu2 = await openMore(user)
    await openSub(user, menu2, "보기 방식")
    await user.click(await screen.findByRole("menuitemradio", { name: "목록" }))
    // Assert
    expect(useViewStore.getState().viewMode).toBe("list")
  })

  it("T-ui-topbar-14 [Happy] the view mode submenu has a 필름스트립 radio that enters the mode; card display is then disabled", async () => {
    // Arrange
    const { user } = await setup()
    const menu = await openMore(user)
    await openSub(user, menu, "보기 방식")
    // Act
    await user.click(await screen.findByRole("menuitemradio", { name: "필름스트립" }))
    // Assert
    expect(useViewStore.getState().viewMode).toBe("filmstrip")
    const menu2 = await openMore(user)
    expect(within(menu2).getByRole("menuitem", { name: "카드 표시" })).toHaveAttribute("aria-disabled", "true")
    await openSub(user, menu2, "보기 방식")
    expect(await screen.findByRole("menuitemradio", { name: "필름스트립" })).toBeChecked()
  })

  it("T-ui-topbar-10 [Happy] the card display submenu reuses CardDisplayMenu's items and a toggle changes the same store", async () => {
    // Arrange
    const { user } = await setup()
    const before = useViewStore.getState().cardDisplay.favorite
    const menu = await openMore(user)
    // Act
    await openSub(user, menu, "카드 표시")
    const item = await screen.findByRole("menuitemcheckbox", { name: "즐겨찾기" })
    // Assert: 같은 항목(초기화·상태 고정 항목 포함)
    expect(screen.getByRole("menuitem", { name: "기본값으로" })).toBeInTheDocument()
    expect(screen.getByRole("menuitemcheckbox", { name: /오프라인·파일 없음/ })).toHaveAttribute("aria-disabled", "true")
    // Act
    await user.click(item)
    // Assert
    expect(useViewStore.getState().cardDisplay.favorite).toBe(!before)
  })

  it("T-ui-topbar-11 [Happy] the settings item calls the same onOpenSettings (opens settings)", async () => {
    // Arrange
    const { user } = await setup()
    expect(useProtectStore.getState().settingsOpen).toBe(false)
    const menu = await openMore(user)
    // Act
    await user.click(within(menu).getByRole("menuitem", { name: "설정" }))
    // Assert
    expect(useProtectStore.getState().settingsOpen).toBe(true)
  })

  it("T-ui-topbar-09 [Business rule, gate ON] semantic search disables the sort and direction items; list mode disables card display", async () => {
    // Arrange
    const { user } = await setup()
    act(() => {
      useSearchStore.getState().setMode("hybrid")
      useSearchStore.getState().setQuery("sunset")
      useViewStore.getState().setViewMode("list")
    })
    // Act
    const menu = await openMore(user)
    // Assert
    expect(within(menu).getByRole("menuitem", { name: "정렬 기준" })).toHaveAttribute("aria-disabled", "true")
    expect(within(menu).getByRole("menuitem", { name: /^정렬 방향/ })).toHaveAttribute("aria-disabled", "true")
    expect(within(menu).getByRole("menuitem", { name: "카드 표시" })).toHaveAttribute("aria-disabled", "true")
    // 보기 방식은 항상 가능
    expect(within(menu).getByRole("menuitem", { name: "보기 방식" })).not.toHaveAttribute("aria-disabled", "true")
  })

  it("T-ui-topbar-12 [Business rule] the update dot and menu item appear only while an update is available, and the item opens the update dialog", async () => {
    // Arrange
    const { user } = await setup()
    expect(within(topBar()).queryByTestId("more-update-dot")).not.toBeInTheDocument()
    let menu = await openMore(user)
    expect(within(menu).queryByRole("menuitem", { name: /업데이트 가능/ })).not.toBeInTheDocument()
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument())
    // Act: 업데이트 발견
    act(() => useUpdateStore.getState().set({ available: UPDATE }))
    // Assert: 점 배지 + 이름에 버전
    expect(within(topBar()).getByTestId("more-update-dot")).toBeInTheDocument()
    expect(moreButton()).toHaveAttribute("title", "더보기 — 업데이트 가능: 1.2.0")
    // Act
    menu = await openMore(user)
    await user.click(within(menu).getByRole("menuitem", { name: "업데이트 가능: 1.2.0" }))
    // Assert
    expect(useUpdateStore.getState().dialogOpen).toBe(true)
  })
})

describe("TopBar 360px breakpoint (container query contract)", () => {
  it("T-ui-topbar-13 [Boundary] below 360px the right group hides and only ⋯ shows; ⋯ is hidden at wider widths", async () => {
    // Arrange + Act
    await setup()
    const bar = topBar()
    const settings = within(bar).getByRole("button", { name: "설정" })
    const group = settings.parentElement as HTMLElement
    // Assert: 정렬·보기·카드 표시·업데이트·설정이 든 묶음이 한 번에 숨는다
    expect(group.className).toContain("@max-[360px]:hidden")
    expect(group.contains(within(bar).getByRole("combobox", { name: "정렬 기준" }))).toBe(true)
    expect(group.contains(within(bar).getByRole("button", { name: "표시 옵션" }))).toBe(true)
    // Assert: ⋯ 는 기본 숨김, 360px 미만에서만 표시
    expect(moreButton().className).toContain("hidden")
    expect(moreButton().className).toContain("@max-[360px]:inline-flex")
    // Assert: 남는 컨트롤(메뉴·검색 아이콘·필터)은 이 묶음 밖이다
    for (const name of ["사이드바 열기/닫기", "검색 열기", "필터"]) expect(group.contains(within(bar).getByRole("button", { name }))).toBe(false)
  })
})
