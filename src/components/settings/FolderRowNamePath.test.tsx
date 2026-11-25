// Categories: Happy (01,02,05,06) / Boundary (03,04,07)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, render, screen, within } from "@testing-library/react"
import { describe, expect, it } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { FolderRow } from "./FolderRow"

async function renderRow(patch: Partial<ManagedFolder>): Promise<HTMLElement> {
  const api = createMockApi({ count: 10 })
  const base = (await api.listFolders())[0]
  const folder: ManagedFolder = { ...base, alias: null, hidden: false, volumeKind: "fixed", ...patch }
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={new QueryClient()}>
        <table>
          <tbody>
            <FolderRow folder={folder} defaultHours={6} onOpenSettings={() => undefined} />
          </tbody>
        </table>
      </QueryClientProvider>
    </ApiProvider>,
  )
  return screen.getByRole("row")
}

describe("FolderRow name + path lines", () => {
  it("T-ui-folderrow-01 [Happy] without alias the name is the last folder; the path line splits into head and last segment", async () => {
    const row = await renderRow({ path: "D:\\Media\\Photos" })
    expect(within(row).getByText("Photos")).toBeInTheDocument() // 이름
    expect(within(row).getByText("D:\\Media")).toBeInTheDocument() // 경로 앞부분
    expect(within(row).getByText("\\Photos")).toBeInTheDocument() // 경로 마지막 세그먼트(구분자 포함)
    expect(within(row).getAllByTitle("D:\\Media\\Photos")).toHaveLength(2) // 이름 + 경로 줄
  })

  it("T-ui-folderrow-02 [Happy] alias wins over the last folder name", async () => {
    const row = await renderRow({ path: "D:\\Media\\Photos", alias: "내 사진" })
    expect(within(row).getByText("내 사진")).toBeInTheDocument()
    expect(within(row).queryByText("Photos")).not.toBeInTheDocument()
    expect(within(row).getByText("\\Photos")).toBeInTheDocument()
  })

  it("T-ui-folderrow-03 [Boundary] UNC path: name is the last segment, head keeps server and share", async () => {
    const row = await renderRow({ path: "\\\\192.168.0.20\\archive\\2024\\원본", volumeKind: "network" })
    expect(within(row).getByText("원본")).toBeInTheDocument()
    expect(within(row).getByText("\\\\192.168.0.20\\archive\\2024")).toBeInTheDocument()
  })

  it("T-ui-folderrow-04 [Boundary] drive root and UNC share root show the full path as the name", async () => {
    const drive = await renderRow({ path: "D:\\" })
    expect(within(drive).getAllByText("D:\\")).toHaveLength(2) // 이름 + 경로 줄 (잘라 나누지 않음)
    cleanup()
    const share = await renderRow({ path: "\\\\server\\share", volumeKind: "network" })
    expect(within(share).getAllByText("\\\\server\\share")).toHaveLength(2)
  })

  it("T-ui-folderrow-05 [Happy] hidden shows an image named 숨김 and no text badge", async () => {
    const row = await renderRow({ hidden: true })
    expect(within(row).getByRole("img", { name: "숨김" })).toBeInTheDocument()
    expect(within(row).queryByText("숨김")).not.toBeInTheDocument()
  })

  it("T-ui-folderrow-06 [Happy] volume kind is a labelled icon for fixed / network / removable, never a text badge", async () => {
    const cases = [
      ["fixed", "내장 저장 장치"],
      ["network", "네트워크 저장 장치"],
      ["removable", "이동식 저장 장치"],
    ] as const
    for (const [kind, label] of cases) {
      const row = await renderRow({ volumeKind: kind })
      expect(within(row).getByRole("img", { name: label })).toBeInTheDocument()
      expect(within(row).queryByText("네트워크")).not.toBeInTheDocument()
      expect(within(row).queryByText("이동식")).not.toBeInTheDocument()
      cleanup()
    }
  })

  it("T-ui-folderrow-07 [Boundary] not hidden: no hidden image", async () => {
    const row = await renderRow({ hidden: false })
    expect(within(row).queryByRole("img", { name: "숨김" })).not.toBeInTheDocument()
  })
})
