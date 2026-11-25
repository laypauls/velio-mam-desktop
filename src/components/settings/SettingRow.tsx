import type { ReactNode } from "react"

/** 라벨·설명 + 오른쪽 컨트롤 한 줄 (스위치 등). */
export function SettingRow({ label, description, children }: { label: string; description?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="text-sm font-medium">{label}</span>
        {description && <span className="text-xs text-muted-foreground break-keep">{description}</span>}
      </div>
      {children}
    </div>
  )
}
