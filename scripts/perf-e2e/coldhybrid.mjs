// usage: node coldhybrid.mjs <exe> <dataDir> <wv2> <runs>  — app start -> first hybrid search (model load incl.), UI-driven
import { spawn } from "node:child_process"
import { connect } from "./cdp.mjs"
import { PAGE_JS, stats, fmt } from "./harness.mjs"
import fs from "node:fs"
const [exe, dataDir, wv2, runsStr, dwellStr = "300"] = process.argv.slice(2)
const corpus = JSON.parse(fs.readFileSync(new URL("./corpus.json", import.meta.url)))
const env = { ...process.env, MAM_DATA_DIR: dataDir, WEBVIEW2_USER_DATA_FOLDER: wv2, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9337" }
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const out = []
for (let i = 0; i < Number(runsStr); i++) {
  const child = spawn(exe, [], { env, stdio: "ignore" })
  let c
  for (let k = 0; k < 400 && !c; k++) { await sleep(50); try { c = await connect(9337) } catch {} }
  for (let k = 0; k < 400; k++) { try { if ((await c.evaluate(`document.querySelectorAll("[role=option]").length`)) > 0) break } catch {} await sleep(50) }
  await c.evaluate(PAGE_JS)
  await c.evaluate(`window.__p4.click("의미")`)
  await sleep(Number(dwellStr))
  const q = corpus.desc[(i * 17) % corpus.desc.length]
  const r = await c.evaluate(`window.__p4.run(${JSON.stringify(q)})`, 30000)
  out.push(r.paint)
  console.log(i, Math.round(r.paint), r.cached ? "cached" : "", r.timedOut ? "timeout" : "")
  c.close()
  try { process.kill(child.pid) } catch {}
  await sleep(2500)
}
console.log("first hybrid query after cold start (ms):", fmt(stats(out)))
