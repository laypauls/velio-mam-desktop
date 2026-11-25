// P5 통합 시험용 — 실제 런타임·워커. 자산 파일 이름을 KV 로, 태그 `auto` 를 돌려준다.
// 파일 이름이 `slow` 로 시작하면 800 ms 돌아(= 호출이 진행 중인 상태를 만든다) 같은 결과를 돌려준다.
mam.providers.register('ping', () => 'pong');
mam.providers.register('meta.extract', (p) => {
  const a = mam.host.call('asset.get', { assetId: p.assetId });
  if (a.fileName.startsWith('slow')) { const t = Date.now(); while (Date.now() - t < 800) {} }
  return { kv: { fileName: a.fileName, revision: p.revision }, tags: ['auto'] };
});
