import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"

/**
 * 시스템(Windows) 음량 상대 변경 — `system_volume_step` 1회 호출(상대 변경이라 연속 입력이 경합하지 않는다).
 * 성공하면 표시기에 변경 후 음량을 띄우고, 실패(출력 장치 없음 등)하면 표시기 오류 문구 + toast(같은 id 로 합쳐 휠 연타에도 하나만).
 * 실패해도 앱 음량으로 대체하지 않는다.
 */
export function useSystemVolume(indicator: { flashSystem: (volume: number) => void; flashError: (message: string) => void }) {
  const api = useApi()
  const { t } = useTranslation()
  return (delta: number) => {
    api.systemVolumeStep(delta).then(
      (volume) => indicator.flashSystem(volume),
      (e: unknown) => {
        const message = t(errorKey(e))
        indicator.flashError(message)
        toast.error(message, { id: "system-volume" })
      }
    )
  }
}
