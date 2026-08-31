// F3: the TOFU host scan has to hit the remote ACTUALLY affected — not blindly
// origin. On a push to "backup" (git@ci.intern:…) github.com (origin) must not
// be scanned, otherwise the retry runs in circles.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteInfo, RepoInfo, ScannedHost } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const scan: ScannedHost = {
  host: "ci.intern",
  changed: false,
  fingerprints: [{ keyType: "ssh-ed25519", sha256: "AAAA" }],
  knownHostsLines: "ci.intern ssh-ed25519 AAAA\n",
};

vi.mock("./api", () => {
  const hostKey = async () => {
    throw { code: "host_key", message: "unknown host key" };
  };
  return {
    api: {
      push: vi.fn(hostKey),
      pushRemote: vi.fn(hostKey),
      cloneFetch: vi.fn(hostKey),
      remotes: vi.fn(async (): Promise<RemoteInfo[]> => [
        { name: "origin", url: "git@github.com:o/r.git" },
        { name: "backup", url: "git@ci.intern:o/r.git" },
      ]),
      sshScanHost: vi.fn(async (): Promise<ScannedHost> => scan),
    },
  };
});

import { api } from "./api";
import { cloneFetchPhase, gitPush, gitPushTo, ui } from "./state.svelte";

const mockedApi = vi.mocked(api);

const repoOf = (path: string): RepoInfo => ({
  path,
  name: "r",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
});

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repoOf("/repo");
  ui.status = null;
  ui.busy = null;
  ui.modal = null;
  ui.error = null;
  ui.info = null;
});

describe("remoteOp host_key (TOFU) picks the right remote", () => {
  it("scans host ci.intern on a push to 'backup' (not github.com)", async () => {
    await gitPushTo("backup");

    expect(mockedApi.sshScanHost).toHaveBeenCalledTimes(1);
    expect(mockedApi.sshScanHost).toHaveBeenCalledWith("ci.intern", null);
    // The TOFU dialog is shown, with the host + port of the affected remote in the modal.
    expect(ui.modal).toMatchObject({ kind: "sshTofu", host: "ci.intern", port: null });
  });

  it("a default push scans the upstream remote, not the first remote", async () => {
    // Upstream = backup/main -> api.push targets 'backup' (ci.intern), even though
    // 'origin' (github.com) is the alphabetically first remote.
    ui.status = { upstream: "backup/main" } as never;

    await gitPush();

    expect(mockedApi.sshScanHost).toHaveBeenCalledWith("ci.intern", null);
    expect(ui.modal).toMatchObject({ kind: "sshTofu", host: "ci.intern" });
  });
});

describe("cloneFetchPhase host_key (TOFU) while cloning", () => {
  it("shows the fingerprint dialog with the host from the clone URL", async () => {
    // Regression for #1: cloneFetch used not to go through the TOFU route, so a
    // first clone to a new SSH host only produced a raw error message and the
    // user had to pre-fill known_hosts through the git CLI.
    const info = repoOf("/repo");
    await cloneFetchPhase(info, "git@ci.intern:o/r.git", { depth: null, blobless: false });

    expect(mockedApi.cloneFetch).toHaveBeenCalledTimes(1);
    // The host comes from the clone URL (not from a remote).
    expect(mockedApi.sshScanHost).toHaveBeenCalledWith("ci.intern", null);
    expect(ui.modal).toMatchObject({ kind: "sshTofu", host: "ci.intern", port: null });
    // The overlay is off again after the dialog opened.
    expect(ui.cloning).toBeNull();
  });
});
