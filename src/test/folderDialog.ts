import { screen, within } from "@testing-library/react"
import type userEvent from "@testing-library/user-event"

/** 설정 > 관리 폴더 목록에서 `path` 폴더의 [설정] 버튼을 눌러 통합 다이얼로그를 열고 돌려준다. */
export async function openFolderSettings(user: ReturnType<typeof userEvent.setup>, settings: HTMLElement, path: string): Promise<HTMLElement> {
  await user.click(await within(settings).findByRole("button", { name: `${path} 설정` }))
  return screen.findByRole("dialog", { name: "관리 폴더 설정" })
}
