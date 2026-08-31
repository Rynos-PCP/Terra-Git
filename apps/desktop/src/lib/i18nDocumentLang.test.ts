// Guards the `<html lang>` sync. index.html ships lang="en", but the attribute
// has to follow the UI language: a screen reader that believes the document is
// German pronounces the English UI with German phonemes (and the other way
// round), and hyphenation and spellcheck follow the same attribute.
//
// The project deliberately runs Vitest without a DOM environment, so this test
// installs a minimal `document` stand-in before importing the module. That is
// also what proves the `typeof document !== "undefined"` guard earns its keep —
// every other test file imports i18n.svelte with no document at all.
import { describe, expect, it } from "vitest";

const html = { lang: "" };
Object.defineProperty(globalThis, "document", {
  value: { documentElement: html },
  configurable: true,
});

// Dynamic, so the stub is in place before the module body runs.
const { i18n, setLang } = await import("./i18n.svelte");

describe("<html lang>", () => {
  it("adopts the language resolved at load time", () => {
    expect(html.lang).toBe(i18n.lang);
  });

  it("follows every switch", () => {
    setLang("de");
    expect(html.lang).toBe("de");
    setLang("en");
    expect(html.lang).toBe("en");
  });
});
