// Generates the third part of THIRD-PARTY-NOTICES.txt: the C libraries that are
// COMPILED INTO the binary from vendored sources.
//
// Why this exists. `cargo about` reads each crate's declared `license` field,
// and for a `-sys` crate that field describes the RUST WRAPPER, not the foreign
// code it carries. libgit2-sys declares "MIT OR Apache-2.0" while the libgit2 C
// sources it compiles are GPL-2.0 with a linking exception; libz-sys declares
// the same while bundling zlib. Both end up inside the shipped binary because
// the workspace builds git2 with `features = ["vendored-libgit2"]`, so both
// licences have to travel with it.
//
// Usage, from the repository root, as the third of three steps:
//   cargo about generate about.hbs > THIRD-PARTY-NOTICES.txt
//   node scripts/gen-npm-notices.mjs >> THIRD-PARTY-NOTICES.txt
//   node scripts/gen-vendored-notices.mjs >> THIRD-PARTY-NOTICES.txt
//
// The list below is deliberately explicit rather than discovered: a vendored C
// library is a legal obligation, not a detail to infer. If a dependency update
// moves or drops one of these files the script FAILS instead of quietly
// emitting an incomplete notice — that is the point.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/** crate → the foreign library it compiles in, and where that library's own licence lives. */
const VENDORED = [
  {
    crate: "libgit2-sys",
    library: "libgit2",
    licenseFile: "libgit2/COPYING",
    spdx: "GPL-2.0-only WITH a linking exception",
    note:
      "Compiled into terra-git from the sources vendored in libgit2-sys\n" +
      "(the workspace builds git2 with the `vendored-libgit2` feature). The linking\n" +
      "exception below is what permits this; it is reproduced in full.",
  },
  {
    crate: "libz-sys",
    library: "zlib",
    licenseFile: "src/zlib/LICENSE",
    spdx: "Zlib",
    note: "Compiled in as libgit2's compression backend, from the sources vendored in libz-sys.",
  },
];

function packages() {
  const raw = execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--all-features"],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  return JSON.parse(raw).packages;
}

const all = packages();
const out = [];
out.push("=".repeat(80));
out.push("Third-party licenses — vendored C libraries");
out.push("=".repeat(80));
out.push("");
out.push("These libraries are compiled into the binary from sources vendored inside a");
out.push("Rust `-sys` crate. Their own licences differ from the crate's declared licence");
out.push("and are reproduced here in full.");
out.push("Generated with `node scripts/gen-vendored-notices.mjs`.");
out.push("");

for (const v of VENDORED) {
  const pkg = all.find((p) => p.name === v.crate);
  if (!pkg) throw new Error(`${v.crate} is no longer in the dependency graph — update this script.`);

  // manifest_path is <src dir>/Cargo.toml
  const dir = pkg.manifest_path.replace(/[/\\]Cargo\.toml$/, "");
  const file = join(dir, v.licenseFile);
  if (!existsSync(file)) {
    throw new Error(
      `${v.library}: expected its licence at ${v.licenseFile} inside ${v.crate} ${pkg.version}, ` +
        `but it is not there. The crate changed its layout — find the file and fix this script.`,
    );
  }

  out.push("-".repeat(80));
  out.push(`${v.library} (${v.spdx}) — via ${v.crate} ${pkg.version}`);
  out.push("");
  out.push(v.note);
  out.push("");
  out.push(readFileSync(file, "utf8").replace(/\r\n/g, "\n").trimEnd());
  out.push("");
}

process.stdout.write(out.join("\n") + "\n");
