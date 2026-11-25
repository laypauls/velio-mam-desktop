// P5 시험용 provider — 자산 메타데이터를 읽어 KV 와 태그를 반환한다 (정상 경로).
mam.providers.register('ping', () => 'pong');
mam.providers.register('meta.extract', (p) => {
  const a = mam.host.call('asset.get', { assetId: p.assetId });
  return { kv: { fileName: a.fileName, revision: p.revision, kind: p.kind }, tags: ['auto'] };
});
