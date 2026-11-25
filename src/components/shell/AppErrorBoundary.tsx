import { TriangleAlert } from "lucide-react"
import { Component, type ErrorInfo, type ReactNode } from "react"
import { withTranslation, type WithTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"

interface State {
  failed: boolean
}

/**
 * 최상위 오류 경계 — 렌더 중 예외가 나도 빈 화면 대신 안내와 "다시 불러오기"를 보인다.
 * (경계가 없으면 React 가 루트를 통째로 언마운트해 창이 비어 보인다.)
 */
class Boundary extends Component<WithTranslation & { children: ReactNode }, State> {
  state: State = { failed: false }

  static getDerivedStateFromError(): State {
    return { failed: true }
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("render failed", error, info.componentStack)
  }

  render() {
    if (!this.state.failed) return this.props.children
    const { t } = this.props
    return (
      <div role="alert" className="flex h-screen flex-col items-center justify-center gap-3 bg-background p-8 text-center text-foreground">
        <TriangleAlert aria-hidden="true" className="size-12 text-muted-foreground" />
        <h1 className="text-base font-medium break-keep">{t("boundary.title")}</h1>
        <p className="max-w-sm text-sm break-keep text-muted-foreground select-text">{t("boundary.description")}</p>
        <Button onClick={() => window.location.reload()}>{t("common.reload")}</Button>
      </div>
    )
  }
}

export const AppErrorBoundary = withTranslation()(Boundary)
