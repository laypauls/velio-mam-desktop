// P5 시험용 provider — 동작을 시험이 호스트 함수 `test.step` 으로 한 호출씩 지정한다
// (null 반환·throw·무한 루프(시간 초과)·워커 종료·비정상 출력 변형).
mam.providers.register('ping', () => 'pong');

function run(id, p) {
  const s = mam.host.call('test.step', { provider: id, asset: p.assetId, revision: p.revision });
  if (s.kill) mam.host.call('test.kill', null);
  if (s.throw) throw new Error(s.throw);
  if (s.spin) { for (;;) {} }
  if (s.bigKv) return { kv: { big: 'x'.repeat(s.bigKv) } };
  if (s.manyKeys) { const kv = {}; for (let i = 0; i < s.manyKeys; i++) kv['k' + i] = i; return { kv }; }
  return s.out === undefined ? null : s.out;
}

['meta.run', 'meta.b', 'ana.c'].forEach((id) => mam.providers.register(id, (p) => run(id, p)));
