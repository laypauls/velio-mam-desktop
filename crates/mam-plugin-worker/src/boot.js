// Evaluated once before the plugin source. Receives the two native functions as arguments (never as globals)
// and returns the worker's private dispatch API. The plugin only sees the frozen `mam` global.
(function (nativeCall, nativeLog) {
  'use strict';
  const commands = Object.create(null);
  const providers = Object.create(null);

  const register = (table, kind) => (id, fn) => {
    if (typeof id !== 'string' || id.length === 0) throw new TypeError(kind + ' id must be a non-empty string');
    if (typeof fn !== 'function') throw new TypeError(kind + ' handler must be a function');
    if (id in table) throw new Error('duplicate ' + kind + ' id: ' + id);
    table[id] = fn;
  };

  const show = (v) => {
    if (typeof v === 'string') return v;
    try { const s = JSON.stringify(v); return s === undefined ? String(v) : s; } catch (_) { return String(v); }
  };
  const logger = (level) => (...args) => { nativeLog(level, args.map(show).join(' ')); };

  const mam = Object.freeze({
    host: Object.freeze({
      // Blocking round trip to the host. Throws Error (with .code) when the host refuses or fails.
      call(method, params) {
        if (typeof method !== 'string' || method.length === 0) throw new TypeError('method must be a non-empty string');
        const raw = nativeCall(method, params === undefined ? 'null' : JSON.stringify(params));
        const r = JSON.parse(raw);
        if (r.ok) return r.value;
        const e = new Error(r.error.message);
        e.code = r.error.code;
        throw e;
      },
    }),
    commands: Object.freeze({ register: register(commands, 'command') }),
    providers: Object.freeze({ register: register(providers, 'provider') }),
    log: Object.freeze({ debug: logger('debug'), info: logger('info'), warn: logger('warn'), error: logger('error') }),
  });
  Object.defineProperty(globalThis, 'mam', { value: mam, enumerable: true });

  return {
    names: () => JSON.stringify({ commands: Object.keys(commands), providers: Object.keys(providers) }),
    dispatch(kind, target, paramsJson) {
      const fn = (kind === 'command' ? commands : providers)[target];
      if (typeof fn !== 'function') throw new Error('not registered: ' + target);
      const r = fn(JSON.parse(paramsJson));
      if (r !== null && typeof r === 'object' && typeof r.then === 'function') {
        throw new TypeError('async handlers are not supported');
      }
      const s = r === undefined ? 'null' : JSON.stringify(r);
      return s === undefined ? 'null' : s;
    },
  };
})
