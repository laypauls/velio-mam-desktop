import { useEffect, useRef } from "react"
import { DetailPanel } from "@/components/detail/DetailPanel"
import { ActionDialogs } from "@/components/actions/ActionDialogs"
import { Gallery } from "@/components/gallery/Gallery"
import { SettingsPage } from "@/components/settings/SettingsPage"
import { ContentViewer } from "@/components/viewer/ContentViewer"
import { useProtectStore, useViewStore } from "@/stores"
import { selectPanelVisible } from "@/stores/view"
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
import { NavRail } from "./NavRail"
import { Sidebar } from "./Sidebar"
import { NewVolumeDialog } from "@/components/volumes/NewVolumeDialog"
import { StartupNoticeHost } from "./StartupNoticeHost"
import { StatusBar } from "./StatusBar"
import { TopBar } from "./TopBar"
import { usePanelAutoCollapse } from "./usePanelAutoCollapse"

/**
 * 레이아웃: [레일(창 세로 전체)] + 오른쪽 열 { 무대(TopBar / 본문 행 | 설정 화면) , 상태 표시줄 }.
 * 본문 행 = 섹션 패널 · (왼쪽 배치) 보기 영역 · 갤러리 · (오른쪽 배치) 보기 영역 · 상세 패널. TopBar 는 보기 영역과 무관하게 항상 같은 자리다.
 * 설정 화면은 무대(레일 오른쪽의 TopBar~본문)만 덮는다 — 레일·상태 표시줄은 유지된다.
 * 라이브러리는 설정 중에도 언마운트하지 않는다 — 검색어·필터·선택·스크롤을 보존하려고 `invisible`(레이아웃·스크롤 유지)
 * + `inert`/`aria-hidden`(포커스·접근성 제외)으로만 가리고, 설정 화면이 위에 겹쳐진다.
 * (`display:none` 은 스크롤 위치를 잃고 폭 0 측정을 만든다.)
 */
export function AppShell() {
  const panelVisible = useViewStore(selectPanelVisible)
  const closePreview = useViewStore((s) => s.closePreview)
  const placement = useViewerPlacement()
  const settingsOpen = useProtectStore((s) => s.settingsOpen)
  const openSettings = useProtectStore((s) => s.openSettings)
  const rowRef = useRef<HTMLDivElement>(null)
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
  usePanelAutoCollapse(rowRef)
  // 콘텐츠 보기 영역이 열린 채 설정으로 가면 보기 영역을 닫는다
  useEffect(() => {
    if (settingsOpen) closePreview()
  }, [settingsOpen, closePreview])
  return (
    <div className="flex h-screen bg-background text-foreground">
      <NavRail />
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div
            inert={settingsOpen}
            aria-hidden={settingsOpen || undefined}
            className={settingsOpen ? "invisible flex min-h-0 flex-1 flex-col" : "flex min-h-0 flex-1 flex-col"}
          >
            <TopBar />
            <div ref={rowRef} className="flex min-h-0 flex-1">
              {panelVisible && <Sidebar />}
              {/* 보기 영역은 왼쪽·오른쪽 배치 모두 이 행 안(갤러리 형제)이라 TopBar 는 움직이지 않는다. 갤러리 최소 폭(288)은 보기 영역 폭 계산(viewerFit)이 보장한다 */}
              {placement === "left" && <ContentViewer />}
              <main className="flex min-w-0 flex-1">
                <Gallery />
              </main>
              {placement === "right" && <ContentViewer />}
              <DetailPanel />
            </div>
          </div>
          {settingsOpen && (
            <div className="absolute inset-0 flex flex-col bg-background">
              <SettingsPage />
            </div>
          )}
        </div>
        <StatusBar onShowErrors={() => openSettings("folders")} />
      </div>
      <ActionDialogs />
      <StartupNoticeHost />
      <NewVolumeDialog />
    </div>
  )
}
