import { Separator } from "@/components/ui/separator"
import { AiCtlGate } from "./ai/AiCtlGate"
import { LocalSection } from "./ai/LocalSection"
import { SimilarSection } from "./ai/SimilarSection"

/** 설정 > 로컬 AI·검색 페이지 — 의미 검색, 자막, 로컬 모델, 유사 미디어 기준. 외부 전송 동의와 무관하다. */
export function AiLocalTab() {
  return (
    <AiCtlGate>
      {(ctl) => (
        <div className="flex flex-col gap-4">
          <LocalSection ctl={ctl} />
          <Separator />
          <SimilarSection />
        </div>
      )}
    </AiCtlGate>
  )
}
