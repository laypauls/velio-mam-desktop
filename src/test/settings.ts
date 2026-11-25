import { screen, within } from "@testing-library/react"
import type userEvent from "@testing-library/user-event"

/**
 * 상단바 톱니로 설정 화면을 열고(선택: 메뉴 항목 이동) 본문 영역(`main`)을 돌려준다.
 * 라이브러리는 aria-hidden 이라 설정 중에는 `main` 이 하나뿐이다.
 */
export async function openSettingsPage(user: ReturnType<typeof userEvent.setup>, menu?: string): Promise<HTMLElement> {
  await user.click(screen.getByRole("button", { name: "설정" }))
  const nav = await screen.findByRole("navigation", { name: "설정 메뉴" })
  if (menu) await user.click(within(nav).getByRole("button", { name: menu }))
  return screen.getByRole("main")
}
