import { Separator } from "@/components/ui/separator"
import { AiCtlGate } from "./ai/AiCtlGate"
import { AutomationSection } from "./ai/AutomationSection"
import { BatchSection } from "./ai/BatchSection"
import { ConsentSection } from "./ai/ConsentSection"
import { ProvidersSection } from "./ai/ProvidersSection"
import { TransferSection } from "./ai/TransferSection"
import { UsageSection } from "./ai/UsageSection"

/** 설정 > 외부 AI 페이지 — 동의, 공급자·키, 자동/예약, 전송 범위, 일괄 분석, 사용량. 로컬 처리는 "로컬 AI·검색" 페이지. */
export function AiTab() {
  return (
    <AiCtlGate>
      {(ctl) => (
        <div className="flex flex-col gap-4">
          <ConsentSection settings={ctl.settings} />
          <Separator />
          <ProvidersSection ctl={ctl} />
          <Separator />
          <AutomationSection ctl={ctl} />
          <Separator />
          <TransferSection ctl={ctl} />
          <Separator />
          <BatchSection />
          <Separator />
          <UsageSection />
        </div>
      )}
    </AiCtlGate>
  )
}
