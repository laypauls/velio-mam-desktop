// Which Rust crates end up in the shipped binaries (pure function over `cargo metadata` output).

const byNameVersion = (a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : a.version < b.version ? -1 : a.version > b.version ? 1 : 0);

/**
 * Crates linked into the shipped binaries: reachable from the workspace members through `normal` dependency edges only
 * (build-dependencies and dev-dependencies are skipped), proc-macro crates excluded (compile-time only, and so is
 * everything only they pull in). A crate reachable through both a normal and a build edge stays. Workspace members are
 * the roots and are not returned. Returns metadata packages sorted by name, version.
 */
export function selectRustPackages(meta) {
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map((meta.resolve?.nodes ?? []).map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const isProcMacro = (p) => (p.targets ?? []).some((t) => (t.kind ?? []).includes("proc-macro"));
  const seen = new Set();
  const queue = [...members];
  while (queue.length) {
    const id = queue.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id)?.deps ?? []) {
      const p = byId.get(d.pkg);
      if (!p || isProcMacro(p)) continue;
      if ((d.dep_kinds ?? []).some((k) => k.kind === null || k.kind === undefined)) queue.push(d.pkg);
    }
  }
  return [...seen]
    .filter((id) => !members.has(id))
    .map((id) => byId.get(id))
    .sort(byNameVersion);
}
