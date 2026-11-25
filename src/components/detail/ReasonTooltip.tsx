import type { ReactNode } from "react"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"

interface Props {
  reason: string | null
  /** 아이콘 전용 버튼의 라벨. 있으면 툴팁 첫 줄에 보이고, 이유가 없어도 라벨 툴팁을 보여준다. */
  label?: string
  children: ReactNode
}

/**
 * 비활성 버튼에 이유를 보여주는 툴팁. 비활성 버튼은 포인터·포커스 이벤트를 받지 못하므로
 * 포커스 가능한 span 이 트리거가 된다. `label` 이 없고 `reason` 도 없으면 children 을 그대로 렌더한다.
 */
export function ReasonTooltip({ reason, label, children }: Props) {
  if (reason === null) {
    if (label === undefined) return <>{children}</>
    return (
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent className="break-keep">{label}</TooltipContent>
      </Tooltip>
    )
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span tabIndex={0} className="inline-flex rounded-md focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none">
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent className="break-keep">
        {label === undefined ? (
          reason
        ) : (
          <div className="flex flex-col gap-0.5">
            <span className="font-medium">{label}</span>
            <span>{reason}</span>
          </div>
        )}
      </TooltipContent>
    </Tooltip>
  )
}
