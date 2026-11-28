import { FileAudio, FileImage, FileVideo, Heart, Library } from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useJobSummary } from "@/hooks/useJobSummary"
import { useSearchStore } from "@/stores"
import { RatingList } from "./RatingGroup"
import { Group, NavItem, sameView } from "./SidebarParts"

const LIBRARY: { kind: "all" | "image" | "video" | "audio" | "favorites"; icon: LucideIcon }[] = [
  { kind: "all", icon: Library },
  { kind: "image", icon: FileImage },
  { kind: "video", icon: FileVideo },
  { kind: "audio", icon: FileAudio },
  { kind: "favorites", icon: Heart },
]

/** "라이브러리" 섹션 패널 — 전체 / 이미지 / 동영상 / 음성 / 즐겨찾기. */
export function SidebarLibrary() {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const jobs = useJobSummary()
  return (
    <Group title={t("sidebar.library")}>
      <ul>
        {LIBRARY.map(({ kind, icon }) => (
          <NavItem
            key={kind}
            icon={icon}
            label={t(`sidebar.view.${kind}`)}
            count={kind === "all" ? jobs.data?.totalAssets : undefined}
            active={sameView(view, { kind })}
            onClick={() => setView({ kind })}
          />
        ))}
      </ul>
    </Group>
  )
}

/** "별점" 섹션 패널 — ★5~★1 · 미평가(개수). 이 컴포넌트가 마운트될 때만 개수를 질의한다. */
export function SidebarRating() {
  const { t } = useTranslation()
  return (
    <Group title={t("sidebar.rating")}>
      <RatingList />
    </Group>
  )
}
