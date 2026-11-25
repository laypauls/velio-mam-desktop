import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"

interface Props {
  icon: LucideIcon
  title: string
  description: string
  action?: ReactNode
  /** 작업이 진행 중인 빈 상태 (`aria-busy`) */
  busy?: boolean
}

/** 빈 상태 — 아이콘 + 타이틀 + 설명 (+ 액션). */
export function EmptyState({ icon: Icon, title, description, action, busy }: Props) {
  return (
    <div role="status" aria-busy={busy || undefined} className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <Icon aria-hidden="true" className="size-12 text-muted-foreground" />
      <h2 className="text-base font-medium break-keep">{title}</h2>
      <p className="max-w-sm text-sm break-keep text-muted-foreground">{description}</p>
      {action}
    </div>
  )
}
