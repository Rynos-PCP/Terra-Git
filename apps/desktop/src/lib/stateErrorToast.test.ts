// The error toast's frame: heading "REPO : code — title",
// the translated text, git's own output as a separate detail, and the ways out
// from the registry.
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
  path: "/work/shop",
  name: "shop",
  currentBranch: "main",
  headDetached: false,
  isEmpty: false,
  historyPrepared: true,
};

vi.mock("./api", () => ({
  api: {
    status: vi.fn(async () => cleanStatus),
    undoStatus: vi.fn(async () => null),
  },
}));

import { errorActions } from "./errorRegistry";
import { t } from "./i18n.svelte";
import { clearToast, setError, showError, showInfo, ui } from "./state.svelte";

beforeEach(() => {
  ui.repo = repo;
  ui.status = cleanStatus;
  clearToast();
});

describe("showError()", () => {
  it("frames the error with repo, code and a short title", () => {
    showError({ code: "non_fast_forward", message: "Push rejected: …" });

    expect(ui.errorMeta).toEqual({
      repo: "shop",
      code: "non_fast_forward",
      title: t("errt.non_fast_forward"),
      detail: null,
    });
    expect(ui.error).toBe(t("err.non_fast_forward"));
  });

  it("keeps git's own output apart from the text", () => {
    showError({
      code: "sidecar_failed",
      message: "git command failed",
      detail: "fatal: Unable to create '.git/index.lock': File exists.",
    });

    expect(ui.error).toBe(t("err.sidecar_failed"));
    expect(ui.errorMeta?.detail).toBe("fatal: Unable to create '.git/index.lock': File exists.");
  });

  it("passes detail-carrying messages through", () => {
    showError({ code: "invalid_operation", message: "The first commit cannot be undone" });

    expect(ui.error).toBe("The first commit cannot be undone");
    expect(ui.errorMeta?.title).toBe(t("errt.invalid_operation"));
  });

  it("names the app instead of a repo when none is open", () => {
    ui.repo = null;
    showError({ code: "network", message: "offline" });

    expect(ui.errorMeta?.repo).toBe("terra-git");
  });

  it("falls back to a generic title for unknown codes", () => {
    showError({ code: "brand_new_code", message: "Something odd" });

    expect(ui.errorMeta?.title).toBe(t("errt.unknown"));
    expect(ui.error).toBe("Something odd");
  });

  it("finds a conflict in git's output, not only in the message", () => {
    showError({
      code: "git_error",
      message: "git command failed",
      detail: "CONFLICT (content): Merge conflict in a.txt",
    });

    expect(ui.errorAction).toEqual({ kind: "conflicts" });
  });
});

describe("the frame is cleared with the toast", () => {
  it("on clearToast and on an info", () => {
    setError("network", "offline");
    showInfo("done");
    expect(ui.errorMeta).toBeNull();

    setError("network", "offline");
    clearToast();
    expect(ui.errorMeta).toBeNull();
  });
});

describe("error registry", () => {
  it("offers both ways out for a full backup store", () => {
    expect(errorActions("discard_store_full")).toEqual(["manageBackups", "raiseLimit"]);
  });

  it("offers nothing for codes without an entry", () => {
    expect(errorActions("merge_conflict")).toEqual([]);
    expect(errorActions(undefined)).toEqual([]);
  });

  it("offers a retry for a network error", () => {
    expect(errorActions("network")).toEqual(["retry"]);
  });
});
