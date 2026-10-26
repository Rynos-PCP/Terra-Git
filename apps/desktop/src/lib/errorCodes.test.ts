// Error parity: every code the app can show has a short
// title and a text in BOTH catalogs, and the registry only names real codes
// and labelled actions. The Rust side checks that errorCodes.ts lists every
// code the backend emits (src-tauri/src/error.rs).
import { describe, expect, it } from "vitest";
import { ERROR_CODES } from "./errorCodes";
import { ERROR_ACTIONS, ERROR_ACTION_LABELS } from "./errorRegistry";
import { de } from "./messages/de";
import { en } from "./messages/en";

const catalogs = { de, en } as Record<string, Record<string, string>>;

describe("error codes", () => {
  it("are sorted and unique", () => {
    expect([...ERROR_CODES]).toEqual([...new Set(ERROR_CODES)].sort());
  });

  for (const [lang, catalog] of Object.entries(catalogs)) {
    it(`each has a title and a text in ${lang}`, () => {
      const missing = ERROR_CODES.flatMap((code) =>
        [`errt.${code}`, `err.${code}`].filter((key) => !(key in catalog)),
      );
      expect(missing).toEqual([]);
    });
  }
});

describe("error registry", () => {
  it("only names known codes", () => {
    const known = new Set<string>(ERROR_CODES);
    expect(Object.keys(ERROR_ACTIONS).filter((c) => !known.has(c))).toEqual([]);
  });

  it("only uses labelled actions, labelled in both catalogs", () => {
    for (const actions of Object.values(ERROR_ACTIONS)) {
      for (const id of actions ?? []) {
        const key = ERROR_ACTION_LABELS[id];
        expect(key in de && key in en, `${id} → ${key}`).toBe(true);
      }
    }
  });
});
