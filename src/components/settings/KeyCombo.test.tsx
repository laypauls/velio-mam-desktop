// Categories: Happy (chips per key, text kept) / Boundary (single key, Escape label)
import { render } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import { KeyCombo } from "./KeyCombo"

describe("KeyCombo", () => {
  it("T-ui-kbd-01 [Happy] a combo renders one keycap per key and the text stays the plain '+' joined notation", () => {
    // Arrange / Act
    const { container } = render(<KeyCombo shortcut="Ctrl+Shift+X" />)
    // Assert
    expect([...container.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["Ctrl", "Shift", "X"])
    expect(container).toHaveTextContent("Ctrl+Shift+X")
  })

  it("T-ui-kbd-02 [Boundary] a single key is one keycap; Escape is shown as Esc", () => {
    // Arrange / Act
    const single = render(<KeyCombo shortcut="[" />)
    const esc = render(<KeyCombo shortcut="Escape" />)
    // Assert
    expect(single.container.querySelectorAll("kbd")).toHaveLength(1)
    expect(esc.container.querySelector("kbd")).toHaveTextContent("Esc")
  })
})
