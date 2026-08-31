// Generates the frontend half of THIRD-PARTY-NOTICES.txt.
//
// `cargo about` walks the Cargo graph only, but the installer also ships the
// built frontend (tauri.conf.json declares `../dist` as frontendDist), so the
// npm packages compiled into that bundle need their notices reproduced too —
// MIT and BSD-3-Clause both require it for binary redistribution.
//
// Usage, from the repository root:
//   cargo about generate about.hbs > THIRD-PARTY-NOTICES.txt
//   node scripts/gen-npm-notices.mjs >> THIRD-PARTY-NOTICES.txt
//
// Which packages count: everything under `dependencies` in
// apps/desktop/package.json (they are imported at runtime) plus svelte, whose
// compiled runtime is part of every component. Build-only tooling (vite,
// eslint, typescript, …) never reaches a user's machine and is left out.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const MODULES = "apps/desktop/node_modules";
const SHIPPED = ["@tauri-apps/api", "@tauri-apps/plugin-dialog", "highlight.js", "svelte"];

/** The licence text as the package itself ships it — never a canned copy. */
function licenseText(pkg) {
  const dir = join(MODULES, pkg);
  const candidates = readdirSync(dir).filter((f) => /^(licen[cs]e|copying)/i.test(f));
  if (candidates.length === 0) return null;
  // A dual-licensed package (Tauri: Apache-2.0 OR MIT) ships both files; keep
  // both, in a stable order, so the notice matches the SPDX expression.
  return candidates
    .sort()
    .map((f) => `--- ${f} ---\n\n${readFileSync(join(dir, f), "utf8").trim()}`)
    .join("\n\n");
}

const out = [];
out.push("=".repeat(80));
out.push("Third-party licenses — frontend (npm)");
out.push("=".repeat(80));
out.push("");
out.push("The installer ships the built frontend. These packages are part of it.");
out.push("Generated with `node scripts/gen-npm-notices.mjs`.");
out.push("");

for (const pkg of SHIPPED) {
  const meta = JSON.parse(readFileSync(join(MODULES, pkg, "package.json"), "utf8"));
  const text = licenseText(pkg);
  out.push("-".repeat(80));
  out.push(`${pkg} ${meta.version} (${meta.license ?? "see below"})`);
  if (meta.homepage) out.push(meta.homepage);
  out.push("");
  out.push(text ?? "(no licence file shipped in the package — see the homepage above)");
  out.push("");
}

process.stdout.write(out.join("\n") + "\n");
