import "@testing-library/jest-dom/vitest"
import { cleanup, configure } from "@testing-library/react"
import { afterEach } from "vitest"

// 전체 스위트가 CPU 부하를 받으면 기본 1000ms 로는 waitFor/findBy 가 간헐 실패한다
configure({ asyncUtilTimeout: 5000 })

// 앱 기본 언어는 en 이지만 기존 컴포넌트 테스트는 한국어 문구를 단언한다 — 테스트 환경의 초기 표시 언어를
// 한국어로 둔다(`@/i18n` 이 이 값을 초기 언어로 읽는다. 기본 en 동작은 src/i18n/initialLanguage.test.ts 가 검증)
if (typeof window !== "undefined") window.localStorage.setItem("mam.uiLanguage", "ko")

afterEach(() => {
  cleanup()
})
