import { beforeEach, describe, expect, it } from "vitest";
import { i18n, resolveErrorMessage, setLang, t, tn } from "./i18n.svelte";
import { de } from "./messages/de";
import { en } from "./messages/en";

beforeEach(() => setLang("de"));

describe("t()", () => {
  it("returns the German message", () => {
    expect(t("app.tab.changes")).toBe("Änderungen");
  });

  it("switches to English without a restart", () => {
    setLang("en");
    expect(i18n.lang).toBe("en");
    expect(t("app.tab.changes")).toBe("Changes");
  });

  it("interpolates parameters", () => {
    setLang("en");
    expect(t("test.greeting", { name: "Ada" })).toBe("Hello Ada!");
    setLang("de");
    expect(t("test.greeting", { name: "Ada" })).toBe("Hallo Ada!");
  });

  it("interpolates the same parameter several times", () => {
    expect(t("test.twice", { x: 7 })).toBe("7 und 7");
  });

  it("falls back to German for a missing en entry, otherwise to the key", () => {
    // @ts-expect-error deliberately unknown key
    expect(t("gibt.es.nicht")).toBe("gibt.es.nicht");
  });
});

describe("tn()", () => {
  it("picks One for n === 1 and Many otherwise, {n} is replaced", () => {
    expect(tn("history.matches", 1)).toBe("1 Treffer");
    expect(tn("history.matches", 5)).toBe("5 Treffer");
    expect(tn("bisect.stepsLeft", 1)).toBe("noch ~1 Schritt");
    expect(tn("bisect.stepsLeft", 3)).toBe("noch ~3 Schritte");
  });

  it("passes additional parameters through (the One variant with {path})", () => {
    expect(tn("changes.discardConfirm", 1, { path: "a.txt" })).toContain("a.txt");
    expect(tn("changes.discardConfirm", 4)).toContain("4");
  });

  it("works after a language change", () => {
    setLang("en");
    expect(tn("state.filesStashed", 1)).toBe("1 file stashed");
    expect(tn("state.filesStashed", 2)).toBe("2 files stashed");
  });
});

describe("resolveErrorMessage()", () => {
  it("translates known backend error codes", () => {
    setLang("en");
    const msg = resolveErrorMessage("non_fast_forward", "deutsche Backend-Meldung");
    expect(msg).not.toBe("deutsche Backend-Meldung");
    expect(msg).toContain("push");
    setLang("de");
    expect(resolveErrorMessage("non_fast_forward", "x")).toContain("Push");
  });

  it("translates the stable pipeline error codes in both languages", () => {
    setLang("en");
    expect(
      resolveErrorMessage("timeout", "Pipeline-Lauf abgebrochen: Zeitlimit erreicht"),
    ).toContain("time limit");
    expect(
      resolveErrorMessage("runner_not_installed", "Pipeline-Runner nicht installiert"),
    ).toContain("not installed");
    expect(resolveErrorMessage("docker_not_running", "Docker laeuft nicht")).toContain("Docker");
    expect(resolveErrorMessage("run_active", "x")).toContain("already active");
    expect(resolveErrorMessage("stage_not_found", "x")).toContain("not found");
    expect(resolveErrorMessage("invalid_target", "x")).toContain("Invalid target");
    expect(resolveErrorMessage("invalid_scope", "x")).toContain("scope");
    setLang("de");
    expect(resolveErrorMessage("timeout", "x")).toContain("Zeitlimit");
    expect(resolveErrorMessage("runner_not_installed", "x")).toContain("gitlab-ci-local");
    expect(resolveErrorMessage("run_active", "x")).toContain("läuft bereits");
    expect(resolveErrorMessage("stage_not_found", "x")).toContain("Stage");
    expect(resolveErrorMessage("invalid_target", "x")).toContain("Ziel");
    expect(resolveErrorMessage("invalid_scope", "x")).toContain("Lauf-Umfang");
    // runner_failed carries details (the runner's stderr) — keep the original.
    expect(resolveErrorMessage("runner_failed", "stderr-Detail")).toBe("stderr-Detail");
    // tools_missing embeds the tool list into the catalog text through {detail}
    // instead of losing it.
    const de_ = resolveErrorMessage("tools_missing", "rsync");
    expect(de_).toContain("rsync");
    expect(de_).toContain("PATH");
    setLang("en");
    const en_ = resolveErrorMessage("tools_missing", "rsync, bash");
    expect(en_).toContain("rsync, bash");
    setLang("de");
  });

  it("falls back to the backend message for unknown codes", () => {
    expect(resolveErrorMessage("voellig_unbekannt", "Original")).toBe("Original");
    expect(resolveErrorMessage(undefined, "Original")).toBe("Original");
    // Codes carrying details stay deliberately untranslated (keep the original).
    expect(resolveErrorMessage("invalid_operation", "Detailtext")).toBe("Detailtext");
  });
});

describe("message catalogs", () => {
  it("de and en have exactly the same keys", () => {
    const deKeys = Object.keys(de).sort();
    const enKeys = Object.keys(en).sort();
    expect(enKeys).toEqual(deKeys);
  });

  it("no empty translations", () => {
    for (const [k, v] of Object.entries(de)) expect(v, `de.${k}`).not.toBe("");
    for (const [k, v] of Object.entries(en)) expect(v, `en.${k}`).not.toBe("");
  });

  // The merge badge in the history was a hard-coded literal for a long time and
  // was therefore the only badge not going through t().
  it("has the merge badge text in de AND en", () => {
    expect(de).toHaveProperty("history.mergeBadge");
    expect(en).toHaveProperty("history.mergeBadge");
    setLang("de");
    expect(t("history.mergeBadge")).toBe("Merge");
    setLang("en");
    expect(t("history.mergeBadge")).toBe("Merge");
  });

  // Stable SSH/clipboard error codes from the backend or the UI: every one of
  // them needs an err.<code> entry in BOTH languages (otherwise a raw message).
  it("has err entries for all SSH backend codes in de AND en", () => {
    const codes = [
      "ssh_no_home",
      "ssh_key_exists",
      "ssh_tool_missing",
      "invalid_key_name",
      "invalid_host",
      "ssh_keyscan_failed",
      "ssh_timeout",
      "ssh_keygen_failed",
      "ssh_untrusted_line",
      "clipboard",
      "ssh_auth",
    ];
    for (const code of codes) {
      const key = `err.${code}`;
      expect(de, `de.${key}`).toHaveProperty(key);
      expect(en, `en.${key}`).toHaveProperty(key);
    }
  });
});
