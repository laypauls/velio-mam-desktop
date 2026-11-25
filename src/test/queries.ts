/**
 * 갤러리 카드(`role="option"`)의 접근 가능한 이름은 `파일명, 유형[, 길이][, 상태]` 다 —
 * 파일명으로 찾을 때 쓰는 `name` 매처. 파일명이 다른 이름의 접두어여도(file_1 / file_10) 구분한다.
 */
export function optionNamed(fileName: string): (accessibleName: string) => boolean {
  return (accessibleName) => accessibleName === fileName || accessibleName.startsWith(`${fileName}, `)
}
