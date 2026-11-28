import type { LucideIcon } from "lucide-react"
import { useId, type ComponentProps, type ReactNode } from "react"
import { Button } from "@/components/ui/button"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

interface Props extends Omit<ComponentProps<typeof Button>, "children" | "size" | "variant"> {
  icon?: LucideIcon
  label: string
  /** 활성 섹션 — 배경 + 왼쪽 막대 */
  active?: boolean
  /** 현재 갤러리 보기가 이 섹션에 속함 — 점 배지 + 보조 설명 */
  dotLabel?: string | null
  children?: ReactNode
}

/** 레일의 아이콘 버튼 — 툴팁(오른쪽) + 포커스 링은 Button 이 준다. 이름은 항상 `label`. */
export function RailButton({ icon: Icon, label, active = false, dotLabel = null, className, children, ...props }: Props) {
  const noteId = useId()
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label={label}
          aria-describedby={dotLabel ? noteId : undefined}
          className={cn(
            // ghost 변형의 aria-expanded 배경(드롭다운 트리거용)을 무력화한다 — 로고·활성 섹션은 aria-expanded 가 켜져 있다
            "relative size-10 text-muted-foreground aria-expanded:bg-transparent aria-expanded:text-muted-foreground",
            active && "bg-sidebar-accent text-foreground aria-expanded:bg-sidebar-accent aria-expanded:text-foreground before:absolute before:inset-y-2.5 before:left-0 before:w-[3px] before:rounded-sm before:bg-primary",
            className
          )}
          {...props}
        >
          {Icon && <Icon aria-hidden="true" className="size-5" />}
          {children}
          {dotLabel && (
            <>
              <span data-testid="rail-view-dot" aria-hidden="true" className="absolute top-1.5 right-1.5 size-1.5 rounded-full bg-primary" />
              <span id={noteId} className="sr-only">
                {dotLabel}
              </span>
            </>
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="right">{dotLabel ? `${label} · ${dotLabel}` : label}</TooltipContent>
    </Tooltip>
  )
}
