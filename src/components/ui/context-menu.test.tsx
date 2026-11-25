// Categories: Validation (T-ui-ctxmenu-01)
import { fireEvent, render, screen } from "@testing-library/react"
import { FolderPlus } from "lucide-react"
import { describe, expect, it } from "vitest"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "./context-menu"

describe("context menu item spacing", () => {
  it("T-ui-ctxmenu-01 [Validation] sub trigger and plain item both keep a gap between icon and text (regression: sub trigger icon glued to its label)", () => {
    // Arrange: jsdom 은 레이아웃이 없다 — 공용 컴포넌트의 클래스 계약(gap-2)으로 확인한다
    render(
      <ContextMenu>
        <ContextMenuTrigger>
          <div>target</div>
        </ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem>
            <FolderPlus aria-hidden="true" />
            일반 항목
          </ContextMenuItem>
          <ContextMenuSub>
            <ContextMenuSubTrigger>
              <FolderPlus aria-hidden="true" />
              하위 메뉴
            </ContextMenuSubTrigger>
            <ContextMenuSubContent>
              <ContextMenuItem>안쪽</ContextMenuItem>
            </ContextMenuSubContent>
          </ContextMenuSub>
        </ContextMenuContent>
      </ContextMenu>
    )
    // Act
    fireEvent.contextMenu(screen.getByText("target"))
    // Assert
    expect(screen.getByRole("menuitem", { name: "일반 항목" })).toHaveClass("gap-2")
    expect(screen.getByRole("menuitem", { name: "하위 메뉴" })).toHaveClass("gap-2")
  })
})
