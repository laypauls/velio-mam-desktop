// usage: node series.mjs <keyword|filter|hybrid|scroll> <N> [tag]
import { open, stats, fmt, save, sleep } from "./harness.mjs"
import fs from "node:fs"
const [kind, nStr, tag = ""] = process.argv.slice(2)
const N = Number(nStr ?? 50)
const corpus = JSON.parse(fs.readFileSync(new URL("./corpus.json", import.meta.url)))
const c = await open()
const ev = (e, t) => c.evaluate(e, t)
const pick = (arr, i, m = 1) => arr[(i * 7919 + m) % arr.length]
let queries = []
if (kind === "keyword" || kind === "filter") {
  const fileTerms = corpus.names.map((n) => n.replace(/\.[^.]+$/, "").split(/[_\- ()]+/)[0]).filter((x) => x.length > 1)
  const terms = [...new Set([...corpus.tags, ...fileTerms])]
  const set = new Set()
  for (let i = 0; set.size < N * 4 && i < 100000; i++) {
    const a = pick(terms, i, 3 + i)
    set.add(i % 3 === 0 ? `${a} ${pick(terms, i, 13)}` : a)
  }
  queries = [...set]
} else if (kind === "hybrid") {
  const s = [...new Set(corpus.desc)]
  queries = s.slice(0, N)
}
// 결과가 있는 질의만 쓴다 (빈 결과 렌더는 첫 페이지 렌더가 아니다)
if (kind === "keyword" || kind === "filter") {
  const f = { mediaTypes: kind === "filter" ? ["image"] : [], includeOffline: true, includeMissing: false, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null, durationMinMs: null, durationMaxMs: null, folder: null, favoritesOnly: false, collectionId: null, tagIds: [] }
  const keep = []
  for (const q of queries) {
    const r = await ev(`window.__p4.ipc(${JSON.stringify({ query: q, filters: f, sort: { field: "capturedAt", direction: "desc" }, limit: 100, cursor: null })})`)
    if (r.n > 0) keep.push(q)
    if (keep.length >= N) break
  }
  queries = keep
}
const out = { kind, tag, when: new Date().toISOString(), runs: [] }
if (kind === "scroll") {
  await ev(`window.__p4.run("")`)
  await sleep(1500)
  for (let i = 0; i < N; i++) {
    out.runs.push(await ev(`window.__p4.scrollNext()`))
    await sleep(150)
  }
  const ok = out.runs.filter((r) => !r.timedOut && !r.cached)
  out.paint = stats(ok.map((r) => r.paint))
  out.dom = stats(ok.map((r) => r.dom))
  out.timeouts = out.runs.filter((r) => r.timedOut).length; out.cachedRuns = out.runs.filter((r) => r.cached).length
  console.log("scroll page fetch+render (ms):", fmt(out.paint), "timeouts", out.timeouts)
} else {
  if (kind === "filter") {
    console.log("click image view:", await ev(`window.__p4.click("이미지")`))
    await sleep(1500)
  }
  if (kind === "hybrid" && tag !== "_img") {
    console.log("all media view:", await ev(`window.__p4.click("전체 미디어")`))
    await sleep(800)
  }
  if (kind === "hybrid") {
    console.log("semantic toggle:", await ev(`window.__p4.click("의미")`))
    await sleep(1500)
  }
  for (const q of ["__warm1", "__warm2"]) {
    await ev(`window.__p4.run(${JSON.stringify(q)})`)
    await ev(`window.__p4.run("")`)
    await sleep(300)
  }
  for (const q of queries) {
    const r = await ev(`window.__p4.run(${JSON.stringify(q)})`)
    const req = {
      query: q,
      filters: { mediaTypes: kind === "filter" ? ["image"] : [], includeOffline: true, includeMissing: false, capturedFrom: null, capturedTo: null, sizeMin: null, sizeMax: null, durationMinMs: null, durationMaxMs: null, folder: null, favoritesOnly: false, collectionId: null, tagIds: [] },
      sort: kind === "hybrid" ? { field: "relevance", direction: "desc" } : { field: "capturedAt", direction: "desc" },
      limit: 100,
      cursor: null,
      ...(kind === "hybrid" ? { mode: "hybrid" } : {}),
    }
    const ipc = await ev(`window.__p4.ipc(${JSON.stringify(req)})`)
    out.runs.push({ q, ...r, ipc })
    await ev(`window.__p4.run("")`)
    await sleep(120)
  }
  const ok = out.runs.filter((r) => !r.timedOut && !r.cached)
  out.paint = stats(ok.map((r) => r.paint))
  out.dom = stats(ok.map((r) => r.dom))
  out.ipc = stats(out.runs.map((r) => r.ipc.ms))
  out.took = stats(out.runs.map((r) => r.ipc.took))
  out.timeouts = out.runs.filter((r) => r.timedOut).length; out.cachedRuns = out.runs.filter((r) => r.cached).length
  out.degraded = out.runs.filter((r) => r.ipc.deg).length
  out.empty = out.runs.filter((r) => r.ipc.n === 0).length
  console.log(`${kind}: UI paint(ms) ${fmt(out.paint)} | dom ${fmt(out.dom)}`)
  console.log(`  IPC(ms) ${fmt(out.ipc)} | backend tookMs ${fmt(out.took)} | timeouts ${out.timeouts} cachedRuns ${out.cachedRuns} degraded ${out.degraded} empty ${out.empty}`)
}
save(kind + tag, out)
c.close()
