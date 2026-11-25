import { connect } from "./cdp.mjs"
import fs from "node:fs"

export const PAGE_JS = `(() => {

  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  const sig = () => {
    const o = document.querySelectorAll("[role=option]");
    if (o.length) return Array.from(o).slice(0, 6).map((e) => e.id).join("|") + "#" + o.length;
    const st = document.querySelector("section [role=status]");
    return "S:" + (st ? st.textContent.slice(0, 40) : "");
  };
  const raf2 = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  const waitChange = (before, t0, timeout, extra) => new Promise((resolve) => {
    let done = false;
    const finish = async (timedOut) => {
      if (done) return; done = true; mo.disconnect(); clearTimeout(to);
      const dom = performance.now() - t0;
      await raf2();
      resolve({ dom, paint: performance.now() - t0, timedOut, sig: sig() });
    };
    const check = () => { const s = sig(); if (s !== before && !s.includes("불러오는") && (!extra || extra())) finish(false); };
    const mo = new MutationObserver(check);
    mo.observe(document.body, { subtree: true, childList: true, attributes: true });
    const to = setTimeout(() => finish(true), timeout);
    check();
  });
  window.__p4 = {
    sig,
    waitChange,
    busy() { const s = document.querySelector("section[aria-label] [tabindex='0']"); return !!s && s.getAttribute("aria-busy") === "true"; },
    // aria-busy(새 결과 대기 중) 가 켜졌다가 꺼질 때까지 — 켜지지 않으면(캐시 적중) 400ms 뒤 cached 로 끝낸다
    async run(q, timeout = 20000) {
      const input = document.querySelector("input[role=searchbox]");
      const before = sig();
      const t0 = performance.now();
      setter.call(input, q);
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      return new Promise((resolve) => {
        let seen = this.busy(), done = false;
        const finish = async (timedOut, cached) => { if (done) return; done = true; mo.disconnect(); clearTimeout(to); clearTimeout(tc); const dom = performance.now() - t0; await raf2(); resolve({ dom, paint: performance.now() - t0, timedOut, cached, sig: sig() }); };
        const check = () => { const b = this.busy(); if (b) seen = true; else if (seen) finish(false, false); };
        const mo = new MutationObserver(check);
        mo.observe(document.body, { subtree: true, childList: true, attributes: true });
        const to = setTimeout(() => finish(true, false), timeout);
        const tc = setTimeout(() => { if (!seen) finish(false, true); }, 400);
        check();
      });
    },
    async ipc(req) {
      const t = performance.now();
      const r = await window.__TAURI_INTERNALS__.invoke("search_assets", { req });
      return { ms: performance.now() - t, took: r.tookMs, n: r.items.length, total: r.total, deg: r.degraded === true, next: r.nextCursor };
    },
    click(label) {
      const el = Array.from(document.querySelectorAll("button,[role=radio],[role=tab]")).find((b) => { const t = (b.getAttribute("aria-label") || b.textContent || "").trim(); return t === label || t.startsWith(label); });
      if (!el) return false; el.click(); return true;
    },
    scroller() { return document.querySelector("section[aria-label] [tabindex='0']"); },
    async scrollNext(timeout = 20000) {
      const s = this.scroller();
      const h0 = s.scrollHeight;
      const t0 = performance.now();
      s.scrollTop = s.scrollHeight;
      return new Promise((resolve) => {
        let done = false;
        const fin = async (timedOut) => { if (done) return; done = true; mo.disconnect(); clearTimeout(to); const dom = performance.now() - t0; await raf2(); resolve({ dom, paint: performance.now() - t0, timedOut, h0, h1: s.scrollHeight }); };
        const check = () => { if (s.scrollHeight > h0) fin(false); };
        const mo = new MutationObserver(check);
        mo.observe(s, { subtree: true, childList: true, attributes: true });
        const to = setTimeout(() => fin(true), timeout);
        check();
      });
    },
  };
  return "installed";
})()`

export function stats(arr) {
  const a = arr.filter((x) => typeof x === "number").sort((x, y) => x - y)
  const q = (p) => a[Math.min(a.length - 1, Math.ceil((p / 100) * a.length) - 1)]
  return { n: a.length, p50: q(50), p95: q(95), p99: q(99), min: a[0], max: a[a.length - 1] }
}

export const fmt = (s) => `n=${s.n} P50=${s.p50.toFixed(0)} P95=${s.p95.toFixed(0)} P99=${s.p99.toFixed(0)} max=${s.max.toFixed(0)}`
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

export async function open() {
  const c = await connect()
  await c.evaluate(PAGE_JS)
  return c
}

export function save(name, obj) {
  fs.mkdirSync(new URL("./results/", import.meta.url), { recursive: true })
  fs.writeFileSync(new URL(`./results/${name}.json`, import.meta.url), JSON.stringify(obj, null, 1))
}
