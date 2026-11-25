import { useId, type ReactNode } from "react"
import { useTranslation } from "react-i18next"

/** 되돌리기 어려운 설정(원본 이동·삭제 사용, 관리 데이터 초기화)을 일반 항목과 구분해 담는 "위험 영역" 블록. */
export function DangerZone({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-4 rounded-xl border border-destructive/40 p-4">
      <h3 id={headingId} className="text-sm font-semibold text-destructive">
        {t("settings.danger.title")}
      </h3>
      {children}
    </section>
  )
}
