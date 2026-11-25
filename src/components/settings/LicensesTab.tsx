import { ChevronRight, FileQuestion } from "lucide-react"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { MarkdownDocument } from "@/components/markdown/MarkdownDocument"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { LICENSES_MD, NOTICES_MD } from "./licenseDocs"
import { parseLicenses } from "./licensesParse"

function EmptyDoc() {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center gap-1 rounded-xl border border-dashed border-border px-4 py-10 text-center">
      <FileQuestion className="size-6 text-muted-foreground" aria-hidden="true" />
      <p className="text-sm font-medium">{t("settings.licenses.emptyTitle")}</p>
      <p className="text-sm text-muted-foreground break-keep">{t("settings.licenses.emptyDesc")}</p>
    </div>
  )
}

interface LicensesTabProps {
  /** 테스트 주입용 — 기본은 빌드에 번들된 루트 문서 */
  notices?: string
  licenses?: string
}

/** 설정 > 오픈소스 라이선스 — 고지(전체 렌더) / 전체 라이선스(절별 접힘, 펼칠 때만 본문 렌더). */
export function LicensesTab({ notices = NOTICES_MD, licenses = LICENSES_MD }: LicensesTabProps) {
  const { t } = useTranslation()
  const parsed = useMemo(() => parseLicenses(licenses), [licenses])
  return (
    <Tabs defaultValue="notices">
      <TabsList>
        <TabsTrigger value="notices">{t("settings.licenses.tabNotices")}</TabsTrigger>
        <TabsTrigger value="licenses">{t("settings.licenses.tabLicenses")}</TabsTrigger>
      </TabsList>
      <TabsContent value="notices">{notices ? <MarkdownDocument source={notices} /> : <EmptyDoc />}</TabsContent>
      <TabsContent value="licenses">
        {parsed.sections.length === 0 ? (
          <EmptyDoc />
        ) : (
          <div className="flex flex-col gap-2">
            {parsed.packageCount !== null && (
              <p className="text-sm text-muted-foreground">{t("settings.licenses.totalPackages", { count: parsed.packageCount })}</p>
            )}
            {parsed.sections.map((s, i) => (
              <Collapsible key={`${i}-${s.title}`} className="rounded-lg border border-border">
                <CollapsibleTrigger className="group flex w-full items-center gap-2 px-3 py-2 text-left text-sm font-medium hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none">
                  <ChevronRight className="size-4 shrink-0 transition-transform group-data-[state=open]:rotate-90" aria-hidden="true" />
                  <span className="min-w-0 break-words">{s.title}</span>
                </CollapsibleTrigger>
                <CollapsibleContent className="border-t border-border px-3 pb-3">
                  <MarkdownDocument source={s.body} />
                </CollapsibleContent>
              </Collapsible>
            ))}
          </div>
        )}
      </TabsContent>
    </Tabs>
  )
}

/** 설정 메뉴 페이지 어댑터 — 저장된 설정은 쓰지 않는다. */
export function LicensesPage() {
  return <LicensesTab />
}
