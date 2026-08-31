import { describe, expect, it } from "vitest";
import { parseSshHost } from "./sshHost";

describe("parseSshHost", () => {
  it("scp form: host, no port", () =>
    expect(parseSshHost("git@192.0.2.10:acme/terra-git.git")).toEqual({
      host: "192.0.2.10",
      port: null,
    }));
  it("scp form github: host, no port", () =>
    expect(parseSshHost("git@github.com:o/r.git")).toEqual({ host: "github.com", port: null }));
  it("ssh:// with a custom port", () =>
    expect(parseSshHost("ssh://git@gitea.example:2222/o/r.git")).toEqual({
      host: "gitea.example",
      port: 2222,
    }));
  it("ssh:// without a port", () =>
    expect(parseSshHost("ssh://git@gitea.example/o/r.git")).toEqual({
      host: "gitea.example",
      port: null,
    }));
  it("ssh:// IPv6 literal without brackets (the backend forms [host]:port)", () =>
    expect(parseSshHost("ssh://git@[2001:db8::1]:22/o/r")).toEqual({
      host: "2001:db8::1",
      port: 22,
    }));
  it("https -> null", () => expect(parseSshHost("https://github.com/o/r.git")).toBeNull());
  it("host:path (scp without a user)", () =>
    expect(parseSshHost("codeberg.org:o/r")).toEqual({ host: "codeberg.org", port: null }));
});
