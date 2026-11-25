import { Copy } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

interface Props {
  /** 접근 가능한 이름이자 툴팁 — "무엇을 복사하는지" 를 밝힌다 */
  label: string
  /** 클립보드에 넣을 문자열 */
  text: string
  /** 성공 toast 번역 키 */
  doneKey: string
  className?: string
}

/** 텍스트(파일명·경로) 복사 아이콘 버튼. 부모에 `group` 을 주면 호버·키보드 포커스 때만 보인다. */
export function CopyTextButton({ label, text, doneKey, className }: Props) {
  const { t } = useTranslation()
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(t(doneKey))
    } catch {
      toast.error(t("detail.copyFailed"))
    }
  }
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      aria-label={label}
      title={label}
      onClick={() => void copy()}
      className={cn("shrink-0 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100", className)}
    >
      <Copy aria-hidden="true" />
    </Button>
  )
}
