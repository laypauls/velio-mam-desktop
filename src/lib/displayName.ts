export interface Titled {
  fileName: string
  title: string | null
}

/** 표시 이름: 제목 편집이 켜져 있고 제목이 있으면 제목, 아니면 파일명(확장자 포함 원문). */
export function displayName(asset: Titled, titleEditEnabled: boolean): string {
  return titleEditEnabled && asset.title ? asset.title : asset.fileName
}

/** 툴팁: 제목이 표시될 때는 "제목 (파일명)", 아니면 파일명. */
export function nameTooltip(asset: Titled, titleEditEnabled: boolean): string {
  const shown = displayName(asset, titleEditEnabled)
  return shown === asset.fileName ? shown : `${shown} (${asset.fileName})`
}

/** `name.ext` → [`name`, `EXT`]. 확장자가 없거나 점으로 시작·끝나는 이름은 [이름, ""] */
export function splitFileName(fileName: string): [string, string] {
  const i = fileName.lastIndexOf(".")
  return i > 0 && i < fileName.length - 1 ? [fileName.slice(0, i), fileName.slice(i + 1).toUpperCase()] : [fileName, ""]
}

/** 제목 입력의 기본값(UI 전용, 저장하지 않음): 저장된 제목이 없으면 확장자를 뺀 파일명. */
export function effectiveTitle(asset: Titled): string {
  return asset.title ?? splitFileName(asset.fileName)[0]
}
