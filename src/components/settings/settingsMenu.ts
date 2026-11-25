import { Brain, Clapperboard, Database, FolderOpen, History, Lock, MonitorCog, Puzzle, Scale, Settings, SlidersHorizontal, Sparkles, type LucideIcon } from "lucide-react"
import type { ComponentType } from "react"
import type { Settings as SettingsData } from "@/lib/ipc-contract"
import type { SettingsSection } from "@/stores"
import { AiLocalTab } from "./AiLocalTab"
import { AiTab } from "./AiTab"
import { DataBackupTab } from "./DataBackupTab"
import { FoldersTab } from "./FoldersTab"
import { GeneralTab } from "./GeneralTab"
import { LicensesPage } from "./LicensesTab"
import { MetadataFieldsTab } from "./MetadataFieldsTab"
import { PlayerSection } from "./PlayerSection"
import { PluginsTab } from "./PluginsTab"
import { ProtectionTab } from "./ProtectionTab"
import { ResumeSection } from "./ResumeSection"
import { ViewBehaviorSection } from "./ViewBehaviorSection"

export interface SettingsMenuItem {
  id: SettingsSection
  /** 메뉴 라벨·페이지 제목 번역 키. 페이지 설명은 `settings.page.<id>` (id 가 하이픈이어도 그대로 키) */
  labelKey: string
  icon: LucideIcon
  /** 페이지 본문 — 저장된 설정을 받는다(필요 없는 페이지는 무시) */
  page: ComponentType<{ settings: SettingsData }>
}

/** 설정 메뉴 정의 한 곳 — 새 페이지 추가 = 항목 1개 + 컴포넌트 1개 + `SettingsSection` 값/번역 키. 배열 순서 = 메뉴 순서. */
export const SETTINGS_MENU: readonly SettingsMenuItem[] = [
  { id: "general", labelKey: "settings.tab.general", icon: Settings, page: GeneralTab },
  { id: "view", labelKey: "settings.tab.view", icon: MonitorCog, page: ViewBehaviorSection },
  { id: "player", labelKey: "settings.tab.player", icon: Clapperboard, page: PlayerSection },
  { id: "resume", labelKey: "settings.tab.resume", icon: History, page: ResumeSection },
  { id: "folders", labelKey: "settings.tab.folders", icon: FolderOpen, page: FoldersTab },
  { id: "metadata", labelKey: "settings.tab.metadata", icon: SlidersHorizontal, page: MetadataFieldsTab },
  { id: "ai", labelKey: "settings.tab.ai", icon: Sparkles, page: AiTab },
  { id: "ai-local", labelKey: "settings.tab.aiLocal", icon: Brain, page: AiLocalTab },
  { id: "protection", labelKey: "settings.tab.protection", icon: Lock, page: ProtectionTab },
  { id: "data", labelKey: "settings.tab.data", icon: Database, page: DataBackupTab },
  { id: "plugins", labelKey: "settings.tab.plugins", icon: Puzzle, page: PluginsTab },
  { id: "licenses", labelKey: "settings.tab.licenses", icon: Scale, page: LicensesPage },
]
