// Recoverable discard: the engine copies ALL affected files
// before every discard. Its storage limit is a hard cap — when a copy does not
// fit, nothing is discarded and the toast offers both ways out. A restore that
// would overwrite files changed since asks first.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RepoInfo, RepoStatus } from "./api";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(async () => false) }));

const cleanStatus: RepoStatus = {
  staged: [],
  unstaged: [],
  branch: "main",
  upstream: null,
  ahead: 0,
  behind: 0,
  opState: "clean",
};

const repo: RepoInfo = {
  path: "/repo",
  name: "repo",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};

vi.mock("./api", () => ({
  api: {
    discard: vi.fn(async () => {}),
    discardHunk: vi.fn(async () => {}),
    restoreDiscarded: vi.fn(async () => {}),
    status: vi.fn(async () => cleanStatus),
    undoStatus: vi.fn(async () => null),
  },
}));

import { confirm } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { formatBytes } from "./format";
import { t } from "./i18n.svelte";
import { discardFiles, discardHunk, restoreDiscarded, ui } from "./state.svelte";

/** The default setting: 500 MB. */
const LIMIT = 500 * 1024 * 1024;
const MB = 1024 * 1024;

const mockedApi = vi.mocked(api);
const mockedConfirm = vi.mocked(confirm);

/** The engine's refusal: "<needed> <used> <limit>" in bytes. */
const storeFull = { code: "discard_store_full", message: `${120 * MB} ${450 * MB} ${LIMIT}` };
const changedSince = { code: "discard_restore_conflict", message: "a.txt" };

beforeEach(() => {
  vi.clearAllMocks();
  ui.repo = repo;
  ui.status = cleanStatus;
  ui.working = 0;
  ui.busy = null;
  ui.error = null;
  ui.info = null;
  ui.errorAction = null;
  ui.discardLimitMb = 500;
  mockedConfirm.mockImplementation(async () => false);
});

describe("discardFiles() on a large set", () => {
  const many = Array.from({ length: 250 }, (_, i) => `f${i}.txt`);

  it("shows a file counter and reports the count", async () => {
    let busyDuring: string | null = null;
    mockedApi.discard.mockImplementationOnce(async (_p, _f, _l, onProgress) => {
      busyDuring = ui.busy;
      onProgress?.({ phase: "backup", percent: 50, done: 125, total: 250 });
      expect(ui.progress?.done).toBe(125);
    });

    await discardFiles(many);

    expect(busyDuring).toBe(t("state.discarding"));
    expect(mockedApi.discard).toHaveBeenCalledWith("/repo", many, LIMIT, expect.any(Function));
    expect(ui.busy).toBeNull();
    expect(ui.info).toBe(t("state.filesDiscardedMany", { n: 250 }));
  });

  it("reports a cancel neutrally — nothing was discarded", async () => {
    mockedApi.discard.mockRejectedValueOnce({ code: "cancelled", message: "cancelled" });

    await discardFiles(many);

    expect(ui.error).toBeNull();
    expect(ui.info).toBe(t("state.discardCancelled"));
  });

  it("keeps the full-store notice with its ways out", async () => {
    mockedApi.discard.mockRejectedValueOnce(storeFull);

    await discardFiles(many);

    expect(ui.errorMeta?.code).toBe("discard_store_full");
  });
});

describe("discardFiles()", () => {
  it("discards with the copy, within the configured limit", async () => {
    await discardFiles(["a.txt"]);

    expect(mockedApi.discard).toHaveBeenCalledTimes(1);
    expect(mockedApi.discard).toHaveBeenCalledWith("/repo", ["a.txt"], LIMIT);
    expect(ui.error).toBeNull();
  });

  it("passes a changed limit setting on", async () => {
    ui.discardLimitMb = 2048;

    await discardFiles(["a.txt"]);

    expect(mockedApi.discard).toHaveBeenCalledWith("/repo", ["a.txt"], 2048 * MB);
  });

  it("stops when the store is full — with the sizes and both ways out", async () => {
    mockedApi.discard.mockRejectedValueOnce(storeFull);

    await discardFiles(["big.bin"]);

    expect(ui.error).toBe(
      t("state.discardStoreFull", {
        needed: formatBytes(120 * MB),
        used: formatBytes(450 * MB),
        limit: formatBytes(LIMIT),
      }),
    );
    expect(ui.errorMeta?.code).toBe("discard_store_full");
    // Never a second attempt without a copy, never a question about it.
    expect(mockedApi.discard).toHaveBeenCalledTimes(1);
    expect(mockedConfirm).not.toHaveBeenCalled();
  });

  it("falls back to a plain text if the sizes are missing", async () => {
    mockedApi.discard.mockRejectedValueOnce({ code: "discard_store_full", message: "" });

    await discardFiles(["big.bin"]);

    expect(ui.error).toBe(t("state.discardStoreFullPlain"));
  });
});

describe("discardHunk()", () => {
  it("reports a full store the same way", async () => {
    mockedApi.discardHunk.mockRejectedValueOnce(storeFull);

    await discardHunk("a.txt", 0);

    expect(ui.errorMeta?.code).toBe("discard_store_full");
  });
});

describe("restoreDiscarded()", () => {
  it("restores and reports it", async () => {
    expect(await restoreDiscarded("files-1-0")).toBe(true);

    expect(mockedApi.restoreDiscarded).toHaveBeenCalledWith("/repo", "files-1-0", LIMIT);
    expect(ui.info).toBe(t("state.discardedRestored"));
  });

  it("never overwrites later changes without asking", async () => {
    mockedApi.restoreDiscarded.mockRejectedValueOnce(changedSince);

    expect(await restoreDiscarded("files-1-0")).toBe(false);

    expect(mockedApi.restoreDiscarded).toHaveBeenCalledTimes(1);
    expect(ui.info).toBeNull();
  });

  it("forces only after the confirmation", async () => {
    mockedApi.restoreDiscarded.mockRejectedValueOnce(changedSince);
    mockedConfirm.mockResolvedValueOnce(true);

    expect(await restoreDiscarded("files-1-0")).toBe(true);

    expect(mockedApi.restoreDiscarded).toHaveBeenLastCalledWith("/repo", "files-1-0", LIMIT, true);
  });

  it("is blocked like a discard when the safety copy does not fit", async () => {
    mockedApi.restoreDiscarded.mockRejectedValueOnce(changedSince);
    mockedConfirm.mockResolvedValueOnce(true);
    mockedApi.restoreDiscarded.mockRejectedValueOnce(storeFull);

    expect(await restoreDiscarded("files-1-0")).toBe(false);

    expect(ui.errorMeta?.code).toBe("discard_store_full");
  });
});
