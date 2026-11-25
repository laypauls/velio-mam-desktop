import type { LucideIcon } from "lucide-react"
import { Play } from "lucide-react"
import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { formatNumber } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SidebarView } from "@/stores"

export function sameView(a: SidebarView, b: SidebarView): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

interface NavItemProps {
  icon: LucideIcon
  label: string
  count?: number
  active: boolean
  /** 연속 재생 중인 항목 — 개수 앞에 ▷ 를 보인다 */
  playing?: boolean
  onClick: () => void
  menu?: ReactNode
}

export function NavItem({ icon: Icon, label, count, active, playing = false, onClick, menu }: NavItemProps) {
  const { t } = useTranslation()
  return (
    <li className="flex items-center gap-0.5">
      <button
        type="button"
        aria-current={active ? "true" : undefined}
        onClick={onClick}
        className={cn("flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted", active && "bg-accent text-accent-foreground")}
      >
        <Icon aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {playing && <Play role="img" aria-label={t("sidebar.playing")} className="size-3 shrink-0 text-primary" />}
        {count !== undefined && <span className="text-xs text-muted-foreground tabular-nums">{formatNumber(count)}</span>}
      </button>
      {menu}
    </li>
  )
}

export function Group({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <div className="flex items-center justify-between px-2">
        <h2 className="text-xs font-medium text-muted-foreground">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  )
}
