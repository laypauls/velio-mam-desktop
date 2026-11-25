/// <reference path="../plugin-api.d.ts" />
// @ts-check
// EXIF 요약 샘플 플러그인 — 메타데이터(asset.read.metadata)를 읽어 KV 요약과 태그를 남긴다.
//  - provider `exif`   : 자산이 새로 색인되거나 바뀔 때 호스트가 자동으로 부른다. 반환값 `{kv, tags}` 를 호스트가 한 트랜잭션으로 기록한다.
//  - command `summarize`: 우클릭 메뉴. 선택한 이미지를 훑어 방향별 개수를 메시지로 돌려주고, 전역 KV `lastSummary` 에 남긴다.
// 문자열은 영어 고정이다(메시지는 호스트 로케일 해석을 거치지 않는다). 메뉴·제목 문구는 l10n/<코드>.json 에 있다.

const HIGH_RES_PIXELS = 8000000;
// 호출당 메시지 예산(10,000) 안에서 끝나도록 명령이 보는 자산 수를 제한한다 (자산당 host 호출 1회).
const MAX_ASSETS = 500;

/** @param {number | null} w @param {number | null} h */
function orientationOf(w, h) {
  if (!w || !h) return null;
  if (w === h) return 'square';
  return w > h ? 'landscape' : 'portrait';
}

/** @param {MamAssetInfo} a */
function summarize(a) {
  const orientation = orientationOf(a.width, a.height);
  const pixels = a.width && a.height ? a.width * a.height : null;
  return {
    fileName: a.fileName,
    width: a.width,
    height: a.height,
    megapixels: pixels === null ? null : Math.round(pixels / 10000) / 100,
    orientation,
    capturedAt: a.capturedAt === null ? null : new Date(a.capturedAt).toISOString(),
    pixels,
  };
}

mam.providers.register('exif', (p) => {
  if (p.kind === 'export') return null;
  const s = summarize(mam.host.call('asset.get', { assetId: p.assetId }));
  const tags = [];
  if (s.orientation) tags.push(s.orientation);
  if (s.pixels !== null && s.pixels >= HIGH_RES_PIXELS) tags.push('high-res');
  const { pixels, ...summary } = s;
  // 태그가 없으면 `tags` 를 생략한다 (빈 배열은 형식 오류).
  return tags.length > 0 ? { kv: { summary }, tags } : { kv: { summary } };
});

mam.commands.register('summarize', (p) => {
  const counts = { landscape: 0, portrait: 0, square: 0, unknown: 0 };
  let images = 0;
  let skipped = 0;
  for (const assetId of p.assetIds.slice(0, MAX_ASSETS)) {
    try {
      const a = mam.host.call('asset.get', { assetId });
      if (a.mediaType !== 'image' || a.status !== 'active') { skipped++; continue; }
      images++;
      counts[orientationOf(a.width, a.height) ?? 'unknown']++;
    } catch (e) {
      // 범위 밖·없는 자산은 not_found. 그 밖의 오류(busy 등)는 다시 던져 호스트가 실패로 처리한다.
      if (/** @type {MamHostError} */ (e).code !== 'not_found') throw e;
      skipped++;
    }
  }
  if (images === 0) return 'No images to summarize';
  mam.host.call('kv.set', { key: 'lastSummary', value: { images, ...counts } });
  const parts = [`${counts.landscape} landscape`, `${counts.portrait} portrait`, `${counts.square} square`];
  if (counts.unknown > 0) parts.push(`${counts.unknown} unknown size`);
  const extra = skipped > 0 ? `, ${skipped} skipped` : '';
  return `Summarized ${images} image(s): ${parts.join(', ')}${extra}`;
});
