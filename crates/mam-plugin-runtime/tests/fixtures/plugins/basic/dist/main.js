// 시험용 플러그인 — 명령·provider 모음
mam.commands.register('echo', (p) => 'done\u0007 ' + p.assetIds.length + ' assets');
mam.commands.register('assets', (p) => p.assetIds);
mam.commands.register('asset', (p) => mam.host.call('asset.get', { assetId: p.assetIds[0] }).fileName);
mam.commands.register('neighbor', (p) => {
  try { mam.host.call('asset.get', { assetId: p.assetIds[0] + 1 }); return 'visible'; } catch (e) { return e.code; }
});
mam.commands.register('setkv', () => {
  try { mam.host.call('kv.set', { key: 'k', value: 1 }); return 'written'; } catch (e) { return e.code; }
});
mam.commands.register('throw', () => { throw new Error('boom'); });
mam.commands.register('flood', () => { for (let i = 0; i < 100000; i++) mam.log.info('spam'); return 1; });
mam.commands.register('slow', () => { const t = Date.now(); while (Date.now() - t < 1200) {} return 'slow done'; });
mam.providers.register('meta.extract', (p) => ({ got: p }));
mam.providers.register('ana.scan', () => null);
mam.providers.register('exp.csv', () => ({ text: 'a,b' }));
