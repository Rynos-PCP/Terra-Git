// Workshop offer in the error toast: conflict candidates have to be recognized
// on every backend path — locale-independently, because git localizes its
// messages ("CONFLICT"/"CONFLIT"/"КОНФЛИКТ") and only the structural gate
// decides on the actual display.
import { describe, expect, it } from "vitest";
import { isConflictCandidate, offerConflictWorkshop } from "./conflictOffer";

describe("isConflictCandidate()", () => {
  it("recognizes the classified pull error by its stable code", () => {
    expect(isConflictCandidate("merge_conflict", "The pull created conflicts — …")).toBe(true);
  });

  it("counts sidecar errors as a candidate locale-independently (French git)", () => {
    expect(
      isConflictCandidate(
        "sidecar_failed",
        "git command failed: CONFLIT (contenu) : Conflit de fusion dans a.txt\n" +
          "La fusion automatique a échoué ; réglez les conflits et validez le résultat.",
      ),
    ).toBe(true);
  });

  it("counts libgit2 errors as a candidate (checkout, stash pop)", () => {
    expect(isConflictCandidate("git_error", "1 conflict prevents checkout")).toBe(true);
  });

  it("catches the remaining paths by the message text (English/German)", () => {
    expect(isConflictCandidate("internal", "Automatic merge failed; fix conflicts …")).toBe(true);
    expect(isConflictCandidate("internal", "… beheben Sie die Konflikte …")).toBe(true);
  });

  it("leaves non-conflict errors untouched", () => {
    expect(isConflictCandidate("network", "Network error: the remote is unreachable.")).toBe(false);
    expect(isConflictCandidate("non_fast_forward", "Push rejected: the remote branch …")).toBe(
      false,
    );
    expect(isConflictCandidate("auth_failed", "Authentication failed.")).toBe(false);
    expect(isConflictCandidate(undefined, undefined)).toBe(false);
  });
});

describe("offerConflictWorkshop()", () => {
  it("offers the workshop during a running operation with open conflicts", () => {
    expect(offerConflictWorkshop("conflicts", "merge", 2, "repo")).toBe(true);
  });

  it("stays silent without a conflict candidate", () => {
    expect(offerConflictWorkshop(null, "merge", 2, "repo")).toBe(false);
  });

  it("stays silent without a running operation (the workshop bounces straight back)", () => {
    expect(offerConflictWorkshop("conflicts", "clean", 2, "repo")).toBe(false);
    expect(offerConflictWorkshop("conflicts", "bisect", 2, "repo")).toBe(false);
  });

  it("stays silent without open conflicted files", () => {
    expect(offerConflictWorkshop("conflicts", "merge", 0, "repo")).toBe(false);
  });

  it("stays silent when the workshop is already open", () => {
    expect(offerConflictWorkshop("conflicts", "merge", 2, "conflicts")).toBe(false);
  });
});
