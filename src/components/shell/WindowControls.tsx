import { Copy, Minus, Square, X } from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { closeWindow, minimizeWindow, toggleMaximizeWindow, watchWindowMaximized } from "@/lib/windowControls"
import { cn } from "@/lib/utils"

function WindowButton({ icon: Icon, label, onClick, danger = false }: { icon: LucideIcon; label: string; onClick: () => void; danger?: boolean }) {
  return (
    <Button
      type="button"
      variant="ghost"
      // 크기를 직접 정한다(공용 size 변형은 정사각형이라 쓰지 않는다) — 칸 높이 = 제목 표시줄 높이
      size={null}
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn("h-full w-11.5 rounded-none", danger && "hover:bg-destructive hover:text-background dark:hover:bg-destructive")}
    >
      <Icon aria-hidden="true" className="size-3.5" />
    </Button>
  )
}

/**
 * 창 버튼 3개(최소화·최대화/복원·닫기) — TopBar 와 설정 헤더가 함께 쓴다.
 * 최대화 아이콘·라벨은 창 크기 변경 이벤트로 맞춘다(Win+↑·스냅·더블클릭도 반영).
 * 그리는 위치에 z-index 를 주지 않는다 — 플레이어 전체 화면(z-40)이 위를 덮어야 한다.
 */
export function WindowControls() {
  const { t } = useTranslation()
  const [maximized, setMaximized] = useState(false)
  useEffect(() => watchWindowMaximized(setMaximized), [])
  return (
    <div role="group" aria-label={t("window.controls")} className="flex h-full shrink-0 items-stretch">
      <WindowButton icon={Minus} label={t("window.minimize")} onClick={() => void minimizeWindow()} />
      <WindowButton
        icon={maximized ? Copy : Square}
        label={t(maximized ? "window.restore" : "window.maximize")}
        onClick={() => void toggleMaximizeWindow()}
      />
      <WindowButton icon={X} label={t("window.close")} onClick={() => void closeWindow()} danger />
    </div>
  )
}
