import { createContext } from "react"

/**
 * 이미지 도구(축소·맞춤·1:1·확대·회전·배율)를 그릴 바깥 자리. 값이 있으면 `ImageViewer` 가 도구 줄을 그 요소로 포털하고(필름스트립 툴바),
 * 없으면 이미지 아래의 떠 있는 도구 줄을 그린다(분할 보기·전체화면).
 */
export const ImageToolsSlotContext = createContext<HTMLElement | null>(null)
