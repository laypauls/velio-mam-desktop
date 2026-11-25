// P5 시험용 내보내기 플러그인 — `exp.step` 은 시험이 `test.step` 으로 출력을 지정하고(text·b64·big·both·none…),
// `exp.echo` 는 받은 자산 목록과 kind 를 그대로 텍스트로 돌려준다. `meta.run` 은 내보내기가 아닌 provider (종류 검증용).
mam.providers.register('ping', () => 'pong');

mam.providers.register('exp.step', (p) => {
  const s = mam.host.call('test.step', { provider: 'exp.step', n: p.assetIds.length });
  if (s.throw) throw new Error(s.throw);
  if (s.bigText) return { text: 'x'.repeat(s.bigText) };
  if (s.bigBytes) return { dataBase64: 'QUJD'.repeat(Math.ceil(s.bigBytes / 3)) }; // 3 바이트 단위 ("ABC")
  return s.out === undefined ? null : s.out;
});

mam.providers.register('exp.echo', (p) => ({ text: p.assetIds.join(',') + '|' + p.kind }));
mam.providers.register('meta.run', () => null);
