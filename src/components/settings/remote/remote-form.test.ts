// Categories: Validation (T-ui-remote-20 required fields, 21 ranges, 23 error mapping) / Boundary (21 ports, 22 secret rules) / Happy (24 toInput)
import { describe, expect, it } from "vitest"
import type { RemoteSource } from "@/lib/ipc-contract"
import { draftOf, emptyDraft, fieldOfError, toInput, validateDraft, type RemoteDraft } from "./remote-form"

const ok = (patch: Partial<RemoteDraft> = {}): RemoteDraft => ({
  ...emptyDraft("ddp_audit"),
  name: "ddp-1",
  host: "10.0.0.5",
  user: "audit",
  secret: "pw",
  pathMap: [{ from: "/", to: "F:\\UHD" }],
  ...patch,
})

const stored: RemoteSource = {
  sourceId: 1,
  kind: "ses_queue",
  name: "ses",
  host: "h",
  port: 22,
  user: "u",
  authKind: "password",
  hasSecret: true,
  hostKey: null,
  logDir: null,
  dbPath: "/srv/ses/events.db",
  pollSec: 3,
  pathMap: [{ from: "/data/incoming", to: "\\\\nas\\media" }],
  createdAt: 1,
  updatedAt: 1,
}

describe("remote source form rules", () => {
  it("T-ui-remote-20 [Validation] an empty new form reports every required field; a complete one has no errors", () => {
    expect(Object.keys(validateDraft(emptyDraft("ddp_audit"), null)).sort()).toEqual(["host", "name", "pathMap", "secret", "user"])
    expect(validateDraft(ok(), null)).toEqual({})
  })

  it.each([
    [{ port: "0" }, "port"],
    [{ port: "65536" }, "port"],
    [{ port: "22.5" }, "port"],
    [{ pollSec: "3601" }, "pollSec"],
    [{ pollSec: "" }, "pollSec"],
    [{ host: "a b" }, "host"],
    [{ logDir: "relative" }, "logDir"],
    [{ pathMap: [{ from: "x", to: "F:\\a" }] }, "pathMap"],
    [{ pathMap: [{ from: "/", to: "\\\\server" }] }, "pathMap"],
    [{ pathMap: [] }, "pathMap"],
  ] as const)("T-ui-remote-21 [Boundary] %j is rejected on %s", (patch, field) => {
    expect(Object.keys(validateDraft(ok(patch as Partial<RemoteDraft>), null))).toEqual([field])
  })

  it("T-ui-remote-21b [Boundary] boundary values 1 / 65535 / 3600 and UNC or drive targets are accepted; a queue needs an absolute DB path", () => {
    expect(validateDraft(ok({ port: "1", pollSec: "3600" }), null)).toEqual({})
    expect(validateDraft(ok({ port: "65535", pathMap: [{ from: "/a", to: "\\\\nas\\media" }] }), null)).toEqual({})
    const queue = ok({ kind: "ses_queue", dbPath: "/srv/ses/events.db" })
    expect(validateDraft(queue, null)).toEqual({})
    expect(Object.keys(validateDraft({ ...queue, dbPath: "events.db" }, null))).toEqual(["dbPath"])
    expect(Object.keys(validateDraft({ ...queue, dbPath: "/srv/dir/" }, null))).toEqual(["dbPath"])
  })

  it("T-ui-remote-22 [Business rule] a secret is optional when editing with the same auth kind, required when the kind changes, and skippable for the fingerprint-only test", () => {
    const edit = draftOf(stored)
    expect(edit.secret).toBe("")
    expect(validateDraft(edit, stored)).toEqual({})
    expect(Object.keys(validateDraft({ ...edit, authKind: "key" }, stored))).toEqual(["secret"])
    expect(Object.keys(validateDraft({ ...edit, authKind: "key", secret: "-----BEGIN" }, stored))).toEqual([])
    // 저장된 비밀이 없는 접속은 다시 입력해야 한다
    expect(Object.keys(validateDraft(edit, { ...stored, hasSecret: false }))).toEqual(["secret"])
    // 지문만 알아 보는 첫 시험은 비밀을 점검하지 않는다
    expect(Object.keys(validateDraft(ok({ secret: "" }), null, false))).toEqual([])
  })

  it("T-ui-remote-23 [Validation] server invalid_input messages map to the form field; other errors map to none", () => {
    expect(fieldOfError({ code: "invalid_input", message: "invalid input: dbPath" })).toBe("dbPath")
    expect(fieldOfError({ code: "invalid_input", message: "invalid input: authKind" })).toBe("secret")
    expect(fieldOfError({ code: "invalid_input", message: "invalid input: kind" })).toBeNull()
    expect(fieldOfError({ code: "busy", message: "invalid input: name" })).toBeNull()
    expect(fieldOfError(new Error("x"))).toBeNull()
  })

  it("T-ui-remote-24 [Happy] toInput trims the name, nulls the unused kind field and an empty secret, and drops nothing else", () => {
    const ddp = toInput(ok({ name: "  ddp-1  ", secret: "", logDir: "" }))
    expect(ddp).toMatchObject({ kind: "ddp_audit", name: "ddp-1", secret: null, logDir: null, dbPath: null, port: 22, pollSec: 3 })
    const queue = toInput(ok({ kind: "ses_queue", dbPath: "/srv/ses/events.db", logDir: "/ignored" }))
    expect(queue).toMatchObject({ dbPath: "/srv/ses/events.db", logDir: null, secret: "pw" })
  })
})
