// Categories: Happy (T-ui-ctxmenu-nat-01) / Boundary (T-ui-ctxmenu-nat-02..04) / Validation (T-ui-ctxmenu-nat-05..06) / Concurrency (T-ui-ctxmenu-nat-07)
import { fireEvent } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"
import { installNativeContextMenuGuard } from "./nativeContextMenu"

let uninstall: (() => void) | null = null
afterEach(() => {
  uninstall?.()
  uninstall = null
  document.body.innerHTML = ""
})

/** 요소를 만들어 body 에 붙이고 우클릭을 보낸다. 기본 동작이 막혔는가(= fireEvent 가 false 반환)를 돌려준다. */
function rightClick(html: string, init: MouseEventInit = {}): boolean {
  document.body.innerHTML = html
  const target = document.body.firstElementChild as Element
  return !fireEvent.contextMenu(target, init)
}

describe("native context menu guard", () => {
  it("T-ui-ctxmenu-nat-01 [Happy] right-click on a plain element is default-prevented", () => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: false })
    // Act + Assert
    expect(rightClick("<div>x</div>")).toBe(true)
  })

  it.each([
    ['<input type="text" />', "text input"],
    ["<input />", "input without type"],
    ['<input type="search" />', "search input"],
    ['<input type="password" />', "password input"],
    ["<textarea></textarea>", "textarea"],
    ['<div contenteditable="true">x</div>', "contenteditable"],
    ['<div contenteditable="true"><b>x</b></div>', "child of contenteditable"],
  ])("T-ui-ctxmenu-nat-02 [Boundary] %s (%s) keeps the native edit menu", (html) => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: false })
    // Act + Assert
    expect(rightClick(html)).toBe(false)
  })

  it.each([
    ['<input type="checkbox" />', "checkbox"],
    ['<input type="range" />', "range"],
    ['<input type="button" value="b" />', "button input"],
    ["<button>b</button>", "button"],
    ["<select><option>a</option></select>", "select"],
    ['<div contenteditable="false">x</div>', "contenteditable=false"],
  ])("T-ui-ctxmenu-nat-03 [Boundary] %s (%s) is suppressed", (html) => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: false })
    // Act + Assert
    expect(rightClick(html)).toBe(true)
  })

  it("T-ui-ctxmenu-nat-04 [Boundary] contenteditable=false nested inside an editable region is suppressed", () => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: false })
    document.body.innerHTML = '<div contenteditable="true"><span id="n" contenteditable="false">x</span></div>'
    // Act
    const allowed = fireEvent.contextMenu(document.getElementById("n") as Element)
    // Assert
    expect(allowed).toBe(false)
  })

  it("T-ui-ctxmenu-nat-05 [Validation] DEV + Shift keeps the native menu (DevTools access)", () => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: true })
    // Act + Assert
    expect(rightClick("<div>x</div>", { shiftKey: true })).toBe(false)
    expect(rightClick("<div>x</div>")).toBe(true) // Shift 없이는 DEV 에서도 막는다
  })

  it("T-ui-ctxmenu-nat-06 [Validation] non-DEV + Shift is still suppressed", () => {
    // Arrange
    uninstall = installNativeContextMenuGuard({ isDev: false })
    // Act + Assert
    expect(rightClick("<div>x</div>", { shiftKey: true })).toBe(true)
  })

  it("T-ui-ctxmenu-nat-07 [Concurrency] installing twice registers one listener; uninstall removes it", () => {
    // Arrange
    let calls = 0
    const probe = () => (calls += 1)
    document.addEventListener("contextmenu", probe)
    installNativeContextMenuGuard({ isDev: false })
    uninstall = installNativeContextMenuGuard({ isDev: false })
    // Act 1: 설치 상태
    const prevented = rightClick("<div>x</div>")
    // Assert 1
    expect(prevented).toBe(true)
    // Act 2: 제거 후
    uninstall()
    uninstall = null
    const afterUninstall = rightClick("<div>x</div>")
    document.removeEventListener("contextmenu", probe)
    // Assert 2
    expect(afterUninstall).toBe(false)
    expect(calls).toBe(2)
  })
})
