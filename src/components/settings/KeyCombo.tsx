import { Fragment } from "react"
import { Kbd } from "@/components/ui/kbd"
import { formatShortcut } from "@/lib/shortcut"

/** 설정 형식 단축키("Ctrl+Shift+X")를 키캡 칩 묶음으로 그린다 — 글자 내용은 "+" 로 이은 원래 표기 그대로라 텍스트로도 읽힌다 */
export function KeyCombo({ shortcut }: { shortcut: string }) {
  const keys = formatShortcut(shortcut).split("+")
  return (
    <span className="inline-flex items-center gap-1">
      {keys.map((k, i) => (
        <Fragment key={i}>
          {i > 0 && <span className="text-muted-foreground">+</span>}
          <Kbd>{k}</Kbd>
        </Fragment>
      ))}
    </span>
  )
}
