import { CircleAlert, WifiOff } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import type { Availability } from "@/lib/ipc-contract"

/** 오프라인·파일 없음 배지 (D6). 정상이면 아무것도 그리지 않는다. */
export function AvailabilityBadge({ availability }: { availability: Availability }) {
  const { t } = useTranslation()
  if (availability === "online") return null
  const Icon = availability === "offline" ? WifiOff : CircleAlert
  return (
    <Badge variant={availability === "offline" ? "secondary" : "destructive"} className="rounded-sm px-1.5 break-keep">
      <Icon aria-hidden="true" className="size-3" />
      {t(`asset.availability.${availability}`)}
    </Badge>
  )
}
