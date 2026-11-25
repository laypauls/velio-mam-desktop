import { useEffect } from "react"
import { DetailPanel } from "@/components/detail/DetailPanel"
import { ActionDialogs } from "@/components/actions/ActionDialogs"
import { Gallery } from "@/components/gallery/Gallery"
import { SettingsPage } from "@/components/settings/SettingsPage"
import { ContentViewer } from "@/components/viewer/ContentViewer"
import { useProtectStore, useViewStore } from "@/stores"
import { useAnalysisEvents } from "@/hooks/useAnalysisEvents"
import { useFolderCountSync } from "@/hooks/useFolderCountSync"
import { useCardDisplayPersistence } from "@/hooks/useCardDisplayPersistence"
import { useLayoutPersistence } from "@/hooks/useLayoutPersistence"
import { usePlayerVolumePersistence } from "@/hooks/usePlayerVolumePersistence"
import { useHideShortcut } from "@/hooks/useHideShortcut"
import { useViewerPlacement } from "@/hooks/useViewerPlacement"
import { usePluginContributionsSync } from "@/hooks/usePluginContributions"
import { useUserFieldFilterSync } from "@/hooks/useUserFieldFilterSync"
import { useViewGuards } from "@/hooks/useViewGuards"
import { useLanguageSync } from "@/i18n/useLanguageSync"
import { useTrayLabelsSync } from "@/i18n/useTrayLabelsSync"
import { useThemeSync } from "@/theme/useThemeSync"
import { Sidebar } from "./Sidebar"
import { NewVolumeDialog } from "@/components/volumes/NewVolumeDialog"
import { StartupNoticeHost } from "./StartupNoticeHost"
import { StatusBar } from "./StatusBar"
import { TopBar } from "./TopBar"

/**
 * 레이아웃: (라이브러리 | 설정 화면) / 상태 표시줄. 두 화면 중 하나만 보이며 상태 표시줄은 항상 유지한다.
 * 라이브러리는 설정 중에도 언마운트하지 않는다 — 검색어·필터·선택·스크롤을 보존하려고 `invisible`(레이아웃·스크롤 유지)
 * + `inert`/`aria-hidden`(포커스·접근성 제외)으로만 가리고, 설정 화면이 위에 겹쳐진다.
 * (`display:none` 은 스크롤 위치를 잃고 폭 0 측정을 만든다.)
 */
export function AppShell() {
  const sidebarOpen = useViewStore((s) => s.sidebarOpen)
  const closePreview = useViewStore((s) => s.closePreview)
  const placement = useViewerPlacement()
  const settingsOpen = useProtectStore((s) => s.settingsOpen)
  const openSettings = useProtectStore((s) => s.openSettings)
  useAnalysisEvents()
  useFolderCountSync()
  useCardDisplayPersistence()
  useLayoutPersistence()
  usePlayerVolumePersistence()
  useThemeSync()
  useLanguageSync()
  useTrayLabelsSync()
  useHideShortcut()
  useViewGuards()
  useUserFieldFilterSync()
  usePluginContributionsSync()
  // 콘텐츠 보기 영역이 열린 채 설정으로 가면 보기 영역을 닫는다
  useEffect(() => {
    if (settingsOpen) closePreview()
  }, [settingsOpen, closePreview])
  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div
          inert={settingsOpen}
          aria-hidden={settingsOpen || undefined}
          className={settingsOpen ? "invisible flex min-h-0 flex-1" : "flex min-h-0 flex-1"}
        >
          {/* 왼쪽 배치: 보기 영역이 창 상단~상태 표시줄까지 세로 전체. 오른쪽 열(TopBar+본문)은 항상 같은 트리 위치라 재마운트되지 않는다 */}
          {placement === "left" && <ContentViewer />}
          <div data-viewer-sibling="" className="flex min-h-0 min-w-0 flex-1 flex-col">
            <TopBar onOpenSettings={() => openSettings()} />
            <div className="flex min-h-0 flex-1">
              {sidebarOpen && <Sidebar />}
              {/* 갤러리 최소 폭(288)은 보기 영역 폭 계산(viewerFit)이 보장한다 — 보기 영역이 최소 폭이 된 뒤에만 이 영역이 더 줄어든다 */}
              <main className="flex min-w-0 flex-1">
                <Gallery />
              </main>
              {placement === "right" && <ContentViewer />}
              <DetailPanel />
            </div>
          </div>
        </div>
        {settingsOpen && (
          <div className="absolute inset-0 flex flex-col bg-background">
            <SettingsPage />
          </div>
        )}
      </div>
      <StatusBar onShowErrors={() => openSettings("folders")} />
      <ActionDialogs />
      <StartupNoticeHost />
      <NewVolumeDialog />
    </div>
  )
}
