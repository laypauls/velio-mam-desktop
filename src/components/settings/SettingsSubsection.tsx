import { useId, type ReactNode } from "react"

/** 소제목(h3) + 선택 설명 — 설정 페이지 안의 모든 소제목이 같은 모양·간격을 쓰도록 하는 공용 블록. */
export function SubsectionHeading({ title, description, id }: { title: string; description?: string; id?: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <h3 id={id} className="text-sm font-semibold">
        {title}
      </h3>
      {description && <p className="text-xs text-muted-foreground break-keep">{description}</p>}
    </div>
  )
}

/** 설정 페이지의 소제목 섹션: `section`(제목으로 이름 지정) = 소제목 + 본문. 페이지는 섹션 사이에 `Separator` 를 둔다. */
export function SettingsSubsection({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  const headingId = useId()
  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-4">
      <SubsectionHeading id={headingId} title={title} description={description} />
      {children}
    </section>
  )
}
