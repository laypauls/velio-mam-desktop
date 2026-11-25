// Categories: Happy (01 02) / Business rule (03 selected color is not muted)
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { describe, expect, it } from "vitest"
import { Toggle } from "./toggle"
import { ToggleGroup, ToggleGroupItem } from "./toggle-group"

const source = (name: string) => readFileSync(resolve(process.cwd(), "src/components/ui", name), "utf8").replace(/\r\n/g, "\n")

describe("shared toggle", () => {
  it("T-ui-toggle-01 [Happy] Toggle exposes pressed state via aria-pressed and data-state", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<Toggle aria-label="t">x</Toggle>)
    const btn = screen.getByRole("button", { name: "t" })
    expect(btn).toHaveAttribute("aria-pressed", "false")
    // Act
    await user.click(btn)
    // Assert
    expect(btn).toHaveAttribute("aria-pressed", "true")
    expect(btn).toHaveAttribute("data-state", "on")
  })

  it("T-ui-toggle-02 [Happy] ToggleGroupItem exposes the selected item", async () => {
    // Arrange
    const user = userEvent.setup()
    render(
      <ToggleGroup type="single" variant="outline" aria-label="g">
        <ToggleGroupItem value="a">A</ToggleGroupItem>
        <ToggleGroupItem value="b">B</ToggleGroupItem>
      </ToggleGroup>,
    )
    // Act
    await user.click(screen.getByRole("radio", { name: "B" }))
    // Assert
    expect(screen.getByRole("radio", { name: "B" })).toHaveAttribute("data-state", "on")
    expect(screen.getByRole("radio", { name: "A" })).toHaveAttribute("data-state", "off")
  })

  it("T-ui-toggle-03 [Business rule] on-state uses primary, never muted (muted ≈ popover in dim/dark)", () => {
    // Arrange / Act
    const toggle = source("toggle.tsx")
    const group = source("toggle-group.tsx")
    // Assert
    expect(toggle).toContain("data-[state=on]:bg-primary")
    expect(toggle).not.toMatch(/aria-pressed:bg-muted|data-\[state=on\]:bg-muted/)
    expect(group).not.toMatch(/data-\[state=on\]:bg-muted/)
  })
})
