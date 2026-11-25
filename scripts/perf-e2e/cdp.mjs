// Minimal CDP client (Node 24 global WebSocket).
export async function connect(port = 9337) {
  const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json()
  const page = list.find((t) => t.type === "page")
  if (!page) throw new Error("no page target")
  const ws = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej })
  let id = 0
  const pending = new Map()
  const events = []
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data)
    if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id) }
    else events.push(d)
  }
  const send = (method, params = {}) => new Promise((res) => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })) })
  const evaluate = async (expression, timeout = 60000) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, timeout })
    if (r.error) throw new Error(JSON.stringify(r.error))
    if (r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 800))
    return r.result.result.value
  }
  return { send, evaluate, events, close: () => ws.close(), page }
}
