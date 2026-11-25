// Categories: Happy (11a 12a 13a 14a) / Boundary (11b 12b 13b 14b) / External (11c folders not loaded)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, within } from "@testing-library/react"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { TooltipProvider } from "@/components/ui/tooltip"
import { AssetInfo } from "@/components/detail/AssetInfo"
import { StatusBar } from "@/components/shell/StatusBar"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { JobSummary, ManagedFolder, ScanError } from "@/lib/ipc-contract"
import { DEFAULT_FILTERS, DEFAULT_SORT } from "@/stores/search"
import { useRelocateFolder } from "@/hooks/useFolderAdmin"
import { RelocateFolderDialog } from "./RelocateFolderDialog"
import { ScanErrorsSection } from "./ScanErrorsSection"

const PATH = "D:\\Photos"
const ALIAS = "내 사진"

afterEach(() => vi.restoreAllMocks())

function renderWith(api: ReturnType<typeof createMockApi>, ui: React.ReactNode) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <TooltipProvider>{ui}</TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
}

async function apiWith(alias: string | null) {
  const api = createMockApi({ count: 10 })
  if (alias) await api.setFolderAlias(1, alias)
  return api
}

function scanSummary(): JobSummary {
  const zero = { pending: 0, running: 0, failed: 0 }
  return {
    paused: false,
    pausedKinds: [],
    counts: { scan: zero, meta: zero, thumb: zero, hash: zero, index: zero, ai: zero, scene: zero, plugin: zero },
    scan: { folderId: 1, folderPath: PATH, processed: 42, total: 100 },
    totalAssets: 0,
  }
}

async function statusBarWith(alias: string | null, foldersLoaded = true) {
  const api = await apiWith(alias)
  vi.spyOn(api, "jobSummary").mockResolvedValue(scanSummary())
  vi.spyOn(api, "onJobsProgress").mockResolvedValue(() => {})
  if (!foldersLoaded) vi.spyOn(api, "listFolders").mockRejectedValue({ code: "internal", message: "x" })
  renderWith(api, <StatusBar />)
  return api
}

describe("status bar scan text", () => {
  it("T-ui-alias-11a [Happy] shows the alias and the full path as tooltip", async () => {
    await statusBarWith(ALIAS)
    const text = await screen.findByText(`스캔 ${ALIAS} 42%`)
    expect(text).toHaveAttribute("title", PATH)
  })

  it("T-ui-alias-11b [Boundary] without alias shows the path and no tooltip", async () => {
    await statusBarWith(null)
    const text = await screen.findByText(`스캔 ${PATH} 42%`)
    expect(text).not.toHaveAttribute("title")
  })

  it("T-ui-alias-11c [External] falls back to the scan path when the folder list is not available", async () => {
    await statusBarWith(ALIAS, false)
    const text = await screen.findByText(`스캔 ${PATH} 42%`)
    expect(text).not.toHaveAttribute("title")
  })
})

describe("asset detail folder line", () => {
  async function renderInfo(alias: string | null) {
    const api = await apiWith(alias)
    const page = await api.searchAssets({ query: "", filters: DEFAULT_FILTERS, sort: DEFAULT_SORT, limit: 5, cursor: null })
    const id = page.items[0].assetId
    const detail = await api.getAssetDetail(id)
    renderWith(api, <AssetInfo data={{ ...detail, folderId: 1 }} />)
    return detail
  }

  it("T-ui-alias-12a [Happy] shows alias › sub-path on one line (full path as tooltip, no separate folder row)", async () => {
    const detail = await renderInfo(ALIAS)
    const line = await screen.findByText(`${ALIAS} › 2026\\trip`)
    expect(line).toHaveAttribute("title", detail.path)
    expect(line.textContent).not.toContain(PATH)
    expect(screen.queryByText("폴더")).not.toBeInTheDocument()
  })

  it("T-ui-alias-12b [Boundary] without alias the location line is the full path", async () => {
    const detail = await renderInfo(null)
    expect(await screen.findByText(detail.path)).toBeInTheDocument()
    expect(screen.queryByText("폴더")).not.toBeInTheDocument()
  })
})

describe("scan errors alias badge", () => {
  async function renderErrors(alias: string | null) {
    const api = await apiWith(alias)
    const error: ScanError = { errorId: 1, folderId: 1, path: `${PATH}\\broken.mov`, errorCode: "ffprobe_failed", message: "x", attempts: 3, lastAt: 1_700_000_000_000 }
    vi.spyOn(api, "listScanErrors").mockResolvedValue({ items: [error], total: 1, nextCursor: null })
    renderWith(api, <ScanErrorsSection />)
    return screen.findByText(error.path)
  }

  it("T-ui-alias-13a [Happy] shows the alias badge on the same line as the error meta", async () => {
    await renderErrors(ALIAS)
    const badge = await screen.findByText(ALIAS)
    expect(badge.parentElement).toHaveTextContent(/ffprobe_failed · 시도 3회/)
  })

  it("T-ui-alias-13b [Boundary] without alias renders no badge", async () => {
    await renderErrors(null)
    expect(screen.getByText(/ffprobe_failed · 시도 3회/)).toBeInTheDocument()
    expect(screen.queryByText(ALIAS)).not.toBeInTheDocument()
  })
})

describe("relocate dialog folder row", () => {
  function Harness({ folder }: { folder: ManagedFolder }) {
    const relocate = useRelocateFolder()
    // 실제 호출부처럼 target 참조를 안정적으로 유지한다(다이얼로그가 target 변경 시 reset)
    const [target] = useState({ folder, newPath: "E:\\Photos2" })
    return <RelocateFolderDialog relocate={relocate} target={target} onClose={() => {}} />
  }

  async function renderDialog(alias: string | null) {
    const api = await apiWith(alias)
    const folder = (await api.listFolders())[0]
    renderWith(api, <Harness folder={folder} />)
    return screen.findByRole("dialog", { name: "관리 폴더 경로 변경" })
  }

  it("T-ui-alias-14a [Happy] shows the alias row and keeps current and new path rows", async () => {
    const dialog = await renderDialog(ALIAS)
    expect(within(dialog).getByText("폴더")).toBeInTheDocument()
    expect(within(dialog).getByText(ALIAS)).toBeInTheDocument()
    expect(within(dialog).getByText(PATH)).toBeInTheDocument()
    expect(within(dialog).getByText("E:\\Photos2")).toBeInTheDocument()
  })

  it("T-ui-alias-14b [Boundary] without alias there is no folder row", async () => {
    const dialog = await renderDialog(null)
    expect(within(dialog).queryByText("폴더")).not.toBeInTheDocument()
    expect(within(dialog).getByText(PATH)).toBeInTheDocument()
  })
})
