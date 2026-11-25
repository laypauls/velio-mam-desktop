import { Copy } from "lucide-react"
import { memo, type ComponentProps } from "react"
import ReactMarkdown, { type Components } from "react-markdown"
import { useTranslation } from "react-i18next"
import remarkGfm from "remark-gfm"
import { toast } from "sonner"
import { cn } from "@/lib/utils"

const SAFE_URL = /^(https?:|mailto:)/i

/** 링크는 이동하지 않는 텍스트 + 복사 버튼 — WebView 안 `<a href>` 클릭은 앱 화면을 외부 페이지로 바꾼다. */
function DocLink({ href, children }: ComponentProps<"a">) {
  const { t } = useTranslation()
  const url = href?.trim() ?? ""
  if (!SAFE_URL.test(url)) return <span>{children}</span>
  const copy = () => {
    void navigator.clipboard
      .writeText(url)
      .then(() => toast.success(t("markdown.linkCopied")))
      .catch(() => toast.error(t("markdown.linkCopyFailed")))
  }
  return (
    <span className="inline-flex items-baseline gap-1">
      <span className="text-primary underline underline-offset-2 break-all" title={url}>
        {children}
      </span>
      <button
        type="button"
        onClick={copy}
        aria-label={t("markdown.copyLink", { url })}
        className="inline-flex size-5 shrink-0 translate-y-0.5 items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <Copy className="size-3" aria-hidden="true" />
      </button>
    </span>
  )
}

const COMPONENTS: Components = {
  h1: ({ children }) => <h1 className="mt-2 mb-3 text-xl font-semibold">{children}</h1>,
  h2: ({ children }) => <h2 className="mt-6 mb-2 border-b border-border pb-1 text-lg font-semibold">{children}</h2>,
  h3: ({ children }) => <h3 className="mt-4 mb-2 text-base font-semibold">{children}</h3>,
  h4: ({ children }) => <h4 className="mt-3 mb-1 text-sm font-semibold">{children}</h4>,
  p: ({ children }) => <p className="my-2 text-sm leading-relaxed">{children}</p>,
  ul: ({ children }) => <ul className="my-2 list-disc space-y-1 pl-6 text-sm">{children}</ul>,
  ol: ({ children }) => <ol className="my-2 list-decimal space-y-1 pl-6 text-sm">{children}</ol>,
  blockquote: ({ children }) => <blockquote className="my-2 border-l-2 border-border pl-3 text-muted-foreground">{children}</blockquote>,
  hr: () => <hr className="my-4 border-border" />,
  a: DocLink,
  img: ({ alt }) => <span>{alt}</span>,
  table: ({ children }) => (
    <div className="relative my-3 overflow-x-auto rounded-lg border border-border">
      <table className="w-full border-collapse text-left text-sm">{children}</table>
    </div>
  ),
  th: ({ children }) => <th className="border-b border-border bg-muted px-3 py-1.5 font-medium">{children}</th>,
  td: ({ children }) => <td className="border-b border-border px-3 py-1.5 align-top">{children}</td>,
  pre: ({ children }) => (
    <pre className="relative my-3 overflow-x-auto rounded-lg bg-muted p-3 text-xs leading-relaxed">{children}</pre>
  ),
  code: ({ className, children }) => (
    <code className={cn("font-mono", !className && "rounded-sm bg-muted px-1 py-0.5 text-[0.85em]", className)}>{children}</code>
  ),
}

const REMARK_PLUGINS = [remarkGfm]

/** 마크다운 문서 렌더러 — 원문 HTML 은 렌더하지 않는다(`skipHtml`, rehype-raw 금지). */
export const MarkdownDocument = memo(function MarkdownDocument({ source, className }: { source: string; className?: string }) {
  return (
    <div className={cn("min-w-0 break-words select-text", className)}>
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS} skipHtml>
        {source}
      </ReactMarkdown>
    </div>
  )
})
