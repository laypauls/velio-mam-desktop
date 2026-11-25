// P5 시험용 두 번째 플러그인 — KV 만 쓴다 (태그를 쓰지 않아 자기 이벤트가 없다).
mam.providers.register('ping', () => 'pong');
mam.providers.register('meta.follow', (p) => ({ kv: { seen: p.revision } }));
