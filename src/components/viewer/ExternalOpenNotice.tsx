import { ExternalLink } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useApi } from "@/lib/api"

interface Props {
  assetId: number
  message: string
  /** 재생 시도가 실패한 경우 — 문구를 알림(role="alert")으로 보인다 */
  isError?: boolean
}

/** 원본을 외부 프로그램으로 여는 버튼 — 실패는 삼킨다 (열기 오류는 OS 가 알린다) */
export function ExternalOpenButton({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const api = useApi()
  return (
    <Button variant="secondary" onClick={() => void api.openOriginal(assetId).catch(() => undefined)}>
      <ExternalLink aria-hidden="true" />
      {t("preview.openExternal")}
    </Button>
  )
}

/** 앱 안에서 재생할 수 없을 때의 안내 — 외부 프로그램으로 열기 버튼을 함께 준다. */
export function ExternalOpenNotice({ assetId, message, isError = false }: Props) {
  return (
    <div role="status" className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 bg-viewer-stage p-6 text-center text-sm text-overlay-foreground">
      <p role={isError ? "alert" : undefined} className="break-keep">
        {message}
      </p>
      <ExternalOpenButton assetId={assetId} />
    </div>
  )
}
