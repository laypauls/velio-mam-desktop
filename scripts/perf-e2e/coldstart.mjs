// usage: node coldstart.mjs <exe> <dataDir> <wv2Dir> <label> [shotPath]
// Process spawn -> first CDP page -> "indexing" status -> first gallery rows painted. Kills the app by PID at the end.
import { spawn } from "node:child_process"
import fs from "node:fs"
import { connect } from "./cdp.mjs"
const [exe, dataDir, wv2, label, shot] = process.argv.slice(2)
const env = { ...process.env, MAM_DATA_DIR: dataDir, WEBVIEW2_USER_DATA_FOLDER: wv2, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9337" }
const t0 = Date.now()
const child = spawn(exe, [], { env, stdio: "ignore", detached: false })
const ev = { label, pid: child.pid }
let c = null
let shotTaken = false
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
while (Date.now() - t0 < 240000) {
  await sleep(40)
  try {
    if (!c) {
      c = await connect(9337)
      ev.cdp = Date.now() - t0
    }
    const st = JSON.parse(await c.evaluate(`JSON.stringify({ rows: document.querySelectorAll("[role=option]").length, status: (document.querySelector("section [role=status]")||{}).textContent||"", boot: !!document.querySelector("[data-boot-splash]"), root: document.getElementById("root") ? document.getElementById("root").childElementCount : -1 })`, 5000))
    if (st.boot && ev.splash === undefined) ev.splash = Date.now() - t0
    if (st.status.includes("색인을 준비") && ev.indexing === undefined) {
      ev.indexing = Date.now() - t0
      if (shot && !shotTaken) {
        shotTaken = true
        const r = await c.send("Page.captureScreenshot", { format: "png" })
        fs.writeFileSync(shot, Buffer.from(r.result.data, "base64"))
      }
    }
    if (st.rows > 0) {
      await c.evaluate(`new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))`)
      ev.firstRows = Date.now() - t0
      ev.rows = st.rows
      break
    }
    ev.last = st
  } catch (e) {
    c = null
  }
}
try { if (c) c.close() } catch {}
try { process.kill(child.pid) } catch {}
await sleep(1500)
console.log(JSON.stringify(ev))
