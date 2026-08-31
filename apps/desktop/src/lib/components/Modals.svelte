<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import type {
    BackupInfo,
    ChangeRequestList,
    CommandError,
    FileDiff,
    SparseStatus,
    SubmoduleInfo,
  } from "../api";
  import { deriveCloneName, timeAgo } from "../format";
  import { t, tn } from "../i18n.svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { parseAutoStashBranch, type SwitchTarget, switchTargetLabel } from "../branchSwitch";
  import type { SwitchFollowUp } from "../state.svelte";
  import {
    addRemote,
    branchFromCommit,
    cancelSshTofu,
    carryChangesAndSwitch,
    cherryPickOnto,
    cloneRepository,
    confirmSshTrust,
    createPrUrl,
    createTag,
    deleteRepoFromDisk,
    deleteTag,
    initRepository,
    prProvider,
    refreshStatus,
    removeRemote,
    renameRemote,
    restoreBackup,
    setRemoteUrl,
    showError,
    showInfo,
    squashFrom,
    stashAndSwitch,
    stashApply,
    stashDrop,
    stashPop,
    stashPush,
    ui,
    updateSubmodules,
  } from "../state.svelte";
  import BlameView from "./BlameView.svelte";
  import ConflictEditor from "./ConflictEditor.svelte";
  import DiffView from "./DiffView.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import RebaseModal from "./RebaseModal.svelte";
  import WorktreesModal from "./WorktreesModal.svelte";

  // Reset all form fields on close (no leaking between modals).
  function close() {
    ui.modal = null;
    tagName = "";
    tagMessage = "";
    squashMessage = "";
    newBranchName = "";
    cloneUrl = "";
    cloneBranch = "";
    cloneName = "";
    nameEdited = false;
    stashMessage = "";
    cpFilter = "";
    remoteName = "";
    remoteUrl = "";
    editingRemote = null;
    submitting = false;
  }

  /** Prevents double execution on a double click on modal actions. */
  let submitting = $state(false);

  // SSH TOFU modal (unknown/changed host key)
  let sshTrustBusy = $state(false);

  async function doConfirmSshTrust() {
    if (sshTrustBusy) return;
    sshTrustBusy = true;
    try {
      await confirmSshTrust();
    } finally {
      sshTrustBusy = false;
    }
  }

  // Clone
  let cloneUrl = $state("");
  let cloneDir = $state("");
  /** Target folder NAME (the folder actually created), pre-filled from the URL
   *  but freely overridable. `nameEdited` stops the auto derivation as soon as
   *  the user has touched the name themselves. */
  let cloneName = $state("");
  let nameEdited = $state(false);
  let cloning = $state(false);
  /** Clone scope: full, blobless (--filter=blob:none) or shallow. */
  let cloneMode = $state<"full" | "blobless" | "shallow">("full");
  let cloneDepth = $state(50);
  /** Optional branch: empty = the remote default, otherwise only this one (single-branch). */
  let cloneBranch = $state("");

  async function pickCloneDir() {
    const dir = await open({ directory: true, title: t("modals.chooseTargetDir") });
    if (typeof dir === "string") cloneDir = dir;
  }

  /** Path join with the separator of the chosen folder (Windows and POSIX paths). */
  function joinPath(dir: string, name: string): string {
    const sep = dir.includes("\\") ? "\\" : "/";
    return dir.replace(/[\\/]+$/, "") + sep + name;
  }

  // As long as the user has not edited the folder name themselves, it follows
  // the value derived from the URL. (It only writes cloneName, never reads it —
  // so there is no feedback loop.)
  $effect(() => {
    if (!nameEdited) cloneName = deriveCloneName(cloneUrl);
  });

  // The ACTUAL target folder = parent folder + the (editable) name.
  const cloneTarget = $derived(
    cloneDir && cloneName.trim() ? joinPath(cloneDir, cloneName.trim()) : "",
  );

  async function doClone() {
    if (!cloneTarget || cloning) return;
    cloning = true;
    const ok = await cloneRepository(cloneUrl.trim(), cloneTarget, {
      depth: cloneMode === "shallow" ? Math.max(1, Math.floor(cloneDepth) || 1) : null,
      blobless: cloneMode === "blobless",
      branch: cloneBranch.trim() || null,
    });
    cloning = false;
    if (ok) {
      cloneUrl = "";
      cloneMode = "full";
      cloneBranch = "";
      close();
    }
  }

  // Init
  let initParent = $state("");
  let initName = $state("");

  async function pickInitDir() {
    const dir = await open({ directory: true, title: t("modals.chooseParentDir") });
    if (typeof dir === "string") initParent = dir;
  }

  async function doInit() {
    if (!initParent || !initName.trim()) return;
    const ok = await initRepository(joinPath(initParent, initName.trim()));
    if (ok) close();
  }

  // Tags / squash / branchFrom / tagAt
  let tagName = $state("");
  let tagMessage = $state("");
  let squashMessage = $state("");
  let newBranchName = $state("");

  async function doCreateTag(target: string) {
    if (!tagName.trim() || submitting) return;
    submitting = true;
    await createTag(tagName.trim(), tagMessage.trim(), target);
    tagName = "";
    tagMessage = "";
    close();
  }

  async function doSquash(oldestId: string) {
    if (!squashMessage.trim() || submitting) return;
    submitting = true;
    await squashFrom(oldestId, squashMessage.trim());
    close();
  }

  async function doBranchFrom(commitId: string) {
    if (!newBranchName.trim() || submitting) return;
    submitting = true;
    await branchFromCommit(newBranchName.trim(), commitId);
    close();
  }

  // Remote management
  let remoteName = $state("");
  let remoteUrl = $state("");
  /** Name of the remote currently being edited (null = none). */
  let editingRemote = $state<string | null>(null);
  let editRemoteName = $state("");
  let editRemoteUrl = $state("");

  async function doAddRemote() {
    if (!remoteName.trim() || !remoteUrl.trim() || submitting) return;
    submitting = true;
    await addRemote(remoteName.trim(), remoteUrl.trim());
    remoteName = "";
    remoteUrl = "";
    submitting = false;
  }

  function startEditRemote(name: string, url: string) {
    editingRemote = name;
    editRemoteName = name;
    editRemoteUrl = url;
  }

  async function doSaveRemote(original: { name: string; url: string }) {
    if (!editRemoteName.trim() || !editRemoteUrl.trim() || submitting) return;
    submitting = true;
    // Order: the URL first (under the old name), then the rename — that way a
    // failure of the second step keeps the URL change.
    if (editRemoteUrl.trim() !== original.url) {
      await setRemoteUrl(original.name, editRemoteUrl.trim());
    }
    if (editRemoteName.trim() !== original.name) {
      await renameRemote(original.name, editRemoteName.trim());
    }
    editingRemote = null;
    submitting = false;
  }

  async function confirmRemoveRemote(name: string) {
    const yes = await confirm(t("modals.removeRemoteConfirm", { name }), {
      title: t("modals.removeRemoteTitle"),
      kind: "warning",
    });
    if (yes) await removeRemote(name);
  }

  async function confirmStashDrop(index: number) {
    const yes = await confirm(t("modals.stashDropConfirm"), {
      title: t("modals.stashDropTitle"),
      kind: "warning",
    });
    if (yes) await stashDrop(index);
  }

  // Create stash (with an optional message)
  let stashMessage = $state("");

  const stashCount = $derived((ui.status?.staged.length ?? 0) + (ui.status?.unstaged.length ?? 0));

  async function doStashPush() {
    if (submitting || stashCount === 0) return;
    submitting = true;
    await stashPush(stashMessage.trim(), []);
    close();
  }

  // Branch switch with uncommitted changes: "bring along" or "leave here"?
  // The branch you are LEAVING — it appears in both labels.
  const switchFrom = $derived(
    ui.repo?.headDetached
      ? t("switch.detachedHead")
      : (ui.status?.branch ?? ui.repo?.currentBranch ?? "—"),
  );

  /** Close the modal first, then switch: the switch runs with its own progress in
   *  the toolbar, and a standing dialog would only cover it.
   *  `andThen` travels along so a composed operation (cherry-pick onto another
   *  branch) rescues its second step across the question. */
  function chooseSwitch(
    run: (target: SwitchTarget, andThen?: SwitchFollowUp) => Promise<void>,
    target: SwitchTarget,
    andThen?: SwitchFollowUp,
  ) {
    close();
    void run(target, andThen);
  }

  // Submodules (load the list when opening, as in the settings modal)
  let subs = $state<SubmoduleInfo[]>([]);
  let subsLoading = $state(false);
  let subsLoaded = $state(false);

  $effect(() => {
    if (ui.modal?.kind === "submodules" && !subsLoaded && ui.repo) {
      subsLoaded = true;
      subsLoading = true;
      api
        .submodules(ui.repo.path)
        .then((list) => (subs = list))
        .catch((e) => {
          showError(e);
        })
        .finally(() => (subsLoading = false));
    }
    if (ui.modal?.kind !== "submodules") subsLoaded = false;
  });

  async function doUpdateSubmodules() {
    await updateSubmodules();
    if (ui.repo) {
      try {
        subs = await api.submodules(ui.repo.path);
      } catch {
        // Keep the list — the error was already reported as a toast.
      }
    }
  }

  // Change requests (PRs/MRs): load the list when opening.
  let crList = $state<ChangeRequestList | null>(null);
  let crError = $state<CommandError | null>(null);
  let crLoading = $state(false);
  let crLoaded = $state(false);

  $effect(() => {
    if (ui.modal?.kind === "changeRequests" && !crLoaded && ui.repo) {
      crLoaded = true;
      loadChangeRequests();
    }
    if (ui.modal?.kind !== "changeRequests") crLoaded = false;
  });

  async function loadChangeRequests() {
    if (!ui.repo) return;
    crLoading = true;
    crError = null;
    try {
      crList = await api.changeRequests(ui.repo.path);
    } catch (e) {
      crError = e as CommandError;
      crList = null;
    } finally {
      crLoading = false;
    }
  }

  const crLabel = $derived(crList?.kind === "gitlab" ? "Merge Requests" : "Pull Requests");

  function crGotoSettings() {
    ui.modal = null;
    ui.view = "settings";
  }

  async function openPrInBrowser() {
    const url = createPrUrl();
    if (url) await api.openExternal(url).catch(() => {});
  }

  // Create a change request (in-app, through the provider API)
  let newCrTitle = $state("");
  let newCrDescription = $state("");
  let newCrTarget = $state("");
  let newCrDraft = $state(false);
  let newCrLoaded = $state(false);
  let newCrError = $state<CommandError | null>(null);

  const newCrSource = $derived(ui.status?.branch ?? null);
  const newCrHasUpstream = $derived(!!ui.status?.upstream);
  const newCrLabel = $derived(prProvider() === "gitlab" ? "Merge Request" : "Pull Request");

  $effect(() => {
    if (ui.modal?.kind === "createCr" && !newCrLoaded && ui.repo) {
      newCrLoaded = true;
      newCrError = null;
      // Pre-fill the title with the newest commit subject, the target branch from the provider.
      newCrTitle = ui.history[0]?.summary ?? "";
      newCrDescription = "";
      newCrDraft = false;
      newCrTarget = "";
      api
        .providerDefaultBranch(ui.repo.path)
        .then((b) => (newCrTarget = newCrTarget || b))
        .catch((e) => (newCrError = e as CommandError));
    }
    if (ui.modal?.kind !== "createCr") newCrLoaded = false;
  });

  async function doCreateCr() {
    if (!ui.repo || !newCrSource || submitting) return;
    if (!newCrTitle.trim() || !newCrTarget.trim()) return;
    submitting = true;
    try {
      const cr = await api.createChangeRequest(ui.repo.path, {
        title: newCrTitle.trim(),
        description: newCrDescription.trim(),
        sourceBranch: newCrSource,
        targetBranch: newCrTarget.trim(),
        draft: newCrDraft,
      });
      showInfo(t("crs.created", { label: newCrLabel, n: cr.number }));
      api.openExternal(cr.webUrl).catch(() => {});
      ui.modal = { kind: "changeRequests" };
    } catch (e) {
      newCrError = e as CommandError;
    } finally {
      submitting = false;
    }
  }

  // Sparse checkout: load the state when opening.
  let sparse = $state<SparseStatus | null>(null);
  const sparseSelected = new SvelteSet<string>();
  let sparseLoading = $state(false);
  let sparseLoaded = $state(false);

  $effect(() => {
    if (ui.modal?.kind === "sparse" && !sparseLoaded && ui.repo) {
      sparseLoaded = true;
      sparseLoading = true;
      api
        .sparseStatus(ui.repo.path)
        .then((s) => {
          sparse = s;
          // Preselection: the active patterns; with sparse inactive, all directories.
          sparseSelected.clear();
          for (const p of s.enabled ? s.patterns : s.topDirs) sparseSelected.add(p);
        })
        .catch((e) => showError(e))
        .finally(() => (sparseLoading = false));
    }
    if (ui.modal?.kind !== "sparse") sparseLoaded = false;
  });

  function toggleSparseDir(dir: string) {
    if (sparseSelected.has(dir)) sparseSelected.delete(dir);
    else sparseSelected.add(dir);
  }

  async function doSparseApply() {
    if (!ui.repo || submitting || sparseSelected.size === 0) return;
    submitting = true;
    try {
      await api.sparseSet(ui.repo.path, [...sparseSelected].sort());
      showInfo(t("sparse.applied"));
      close();
      await refreshStatus();
    } catch (e) {
      showError(e);
      submitting = false;
    }
  }

  async function doSparseDisable() {
    if (!ui.repo || submitting) return;
    submitting = true;
    try {
      await api.sparseDisable(ui.repo.path);
      showInfo(t("sparse.disabled"));
      close();
      await refreshStatus();
    } catch (e) {
      showError(e);
      submitting = false;
    }
  }

  // Backups (backup refs): load the list when opening.
  let backups = $state<BackupInfo[]>([]);
  let backupsLoading = $state(false);
  let backupsLoaded = $state(false);

  $effect(() => {
    if (ui.modal?.kind === "backups" && !backupsLoaded && ui.repo) {
      backupsLoaded = true;
      backupsLoading = true;
      api
        .backups(ui.repo.path)
        .then((list) => (backups = list))
        .catch((e) => {
          showError(e);
        })
        .finally(() => (backupsLoading = false));
    }
    if (ui.modal?.kind !== "backups") backupsLoaded = false;
  });

  /** Display name of the triggering operation. */
  function backupOpLabel(op: string): string {
    const labels: Record<string, string> = {
      squash: "Squash",
      rebase: "Rebase",
      "rebase-interactive": t("modals.backupOpRebaseInteractive"),
      restore: t("modals.backupOpRestore"),
    };
    return labels[op] ?? op;
  }

  async function doRestoreBackup(b: BackupInfo) {
    const yes = await confirm(
      t("modals.restoreBackupConfirm", { subject: b.subject, id: b.targetId.slice(0, 8) }),
      { title: t("modals.restoreBackupTitle"), kind: "warning" },
    );
    if (!yes || !ui.repo) return;
    const ok = await restoreBackup(b.name);
    if (ok) {
      backups = await api.backups(ui.repo.path).catch(() => backups);
    }
  }

  async function doDeleteBackup(b: BackupInfo) {
    const yes = await confirm(t("modals.deleteBackupConfirm", { op: backupOpLabel(b.op) }), {
      title: t("modals.deleteBackupTitle"),
      kind: "warning",
    });
    if (!yes || !ui.repo) return;
    try {
      await api.deleteBackup(ui.repo.path, b.name);
      backups = backups.filter((x) => x.name !== b.name);
    } catch (e) {
      showError(e);
    }
  }

  // Stash preview: load the diff of the stash commit against its base.
  let stashDiff = $state<FileDiff[] | null>(null);
  let stashDiffLoaded = $state(false);

  $effect(() => {
    if (ui.modal?.kind === "stashPreview" && !stashDiffLoaded && ui.repo) {
      stashDiffLoaded = true;
      stashDiff = null;
      api
        .commitDiff(ui.repo.path, ui.modal.id)
        .then((d) => (stashDiff = d))
        .catch((e) => {
          showError(e);
          stashDiff = [];
        });
    }
    if (ui.modal?.kind !== "stashPreview") {
      stashDiffLoaded = false;
      stashDiff = null;
    }
  });

  // Cherry-pick onto another branch
  let cpFilter = $state("");

  const cpBranches = $derived.by(() => {
    const locals = ui.branches.filter((b) => !b.isRemote && !b.isHead);
    const f = cpFilter.trim().toLowerCase();
    return f ? locals.filter((b) => b.name.toLowerCase().includes(f)) : locals;
  });

  function doCherryPickTo(branch: string) {
    const commitId = (ui.modal as { commitId: string }).commitId;
    close();
    cherryPickOnto(commitId, branch);
  }
</script>

{#if ui.modal?.kind === "clone"}
  <Modal title={t("modals.cloneTitle")} onclose={() => !cloning && close()}>
    <label>
      <span class="lbl">{t("modals.cloneUrlLabel")}</span>
      <input type="text" placeholder={t("modals.cloneUrlPlaceholder")} bind:value={cloneUrl} />
    </label>
    <label>
      <span class="lbl">{t("modals.targetDirLabel")}</span>
      <div class="row">
        <input type="text" placeholder={t("modals.dirPlaceholder")} bind:value={cloneDir} />
        <button onclick={pickCloneDir} aria-label={t("modals.chooseTargetDir")}>
          <Icon name="folder" size={14} />
        </button>
      </div>
    </label>
    <label>
      <span class="lbl">{t("modals.cloneNameLabel")}</span>
      <input
        type="text"
        placeholder={t("modals.cloneNamePlaceholder")}
        bind:value={cloneName}
        oninput={() => (nameEdited = true)}
      />
    </label>
    <label>
      <span class="lbl">{t("clone.mode")}</span>
      <div class="row">
        <select bind:value={cloneMode}>
          <option value="full">{t("clone.full")}</option>
          <option value="blobless">{t("clone.blobless")}</option>
          <option value="shallow">{t("clone.shallow")}</option>
        </select>
        {#if cloneMode === "shallow"}
          <input
            type="number"
            min="1"
            style="max-width: 110px"
            aria-label={t("clone.depthLabel")}
            bind:value={cloneDepth}
          />
        {/if}
      </div>
    </label>
    <label>
      <span class="lbl">{t("clone.branchLabel")}</span>
      <input type="text" placeholder={t("clone.branchPlaceholder")} bind:value={cloneBranch} />
    </label>
    {#if cloneMode === "blobless"}
      <p class="hint">{t("clone.bloblessHint")}</p>
    {:else if cloneMode === "shallow"}
      <p class="hint">{t("clone.shallowHint")}</p>
    {/if}
    {#if cloneTarget}
      <p class="hint">{t("modals.cloneTargetHint")} <code>{cloneTarget}</code></p>
    {/if}
    {#if cloning}
      <p class="hint"><span class="spin"></span> {t("modals.cloningHint")}</p>
    {/if}
    <p class="hint">{t("modals.cloneAuthHint")}</p>
    <div class="actions">
      <button disabled={cloning} onclick={close}>{t("common.cancel")}</button>
      <button class="primary" disabled={!cloneTarget || cloning} onclick={doClone}>
        {#if cloning}<span class="spin"></span>{/if}
        {t("modals.cloneAction")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "init"}
  <Modal title={t("modals.initTitle")} onclose={close}>
    <label>
      <span class="lbl">{t("modals.nameLabel")}</span>
      <input type="text" placeholder={t("modals.initNamePlaceholder")} bind:value={initName} />
    </label>
    <label>
      <span class="lbl">{t("modals.parentDirLabel")}</span>
      <div class="row">
        <input type="text" placeholder={t("modals.dirPlaceholder")} bind:value={initParent} />
        <button onclick={pickInitDir} aria-label={t("modals.chooseParentDir")}>
          <Icon name="folder" size={14} />
        </button>
      </div>
    </label>
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <button class="primary" disabled={!initParent || !initName.trim()} onclick={doInit}>
        {t("common.create")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "stash"}
  <Modal title={t("modals.stashesTitle")} onclose={close}>
    {#each ui.stashes as stash (stash.id)}
      <!-- Changes left behind during a branch switch carry a
           technical marker (branchSwitch.ts) — the readable version stands
           here, the raw text stays in the tooltip. -->
      {@const leftOn = parseAutoStashBranch(stash.message)}
      <div class="list-row">
        <Icon name="stash" size={14} />
        <span class="grow" title={stash.message}>
          {leftOn ? t("modals.autoStashLabel", { name: leftOn }) : stash.message}
        </span>
        <button
          class="ghost"
          title={t("modals.viewContents")}
          onclick={() =>
            (ui.modal = { kind: "stashPreview", id: stash.id, message: stash.message })}
        >
          <Icon name="eye" size={13} />
        </button>
        <button
          class="ghost"
          disabled={ui.working > 0}
          title={t("modals.applyKeep")}
          onclick={() => stashApply(stash.index)}
        >
          {t("modals.apply")}
        </button>
        <button
          class="ghost"
          disabled={ui.working > 0}
          title={t("modals.applyRemove")}
          onclick={() => stashPop(stash.index)}
        >
          {t("modals.pop")}
        </button>
        <button
          class="ghost danger"
          disabled={ui.working > 0}
          title={t("modals.discard")}
          onclick={() => confirmStashDrop(stash.index)}
        >
          <Icon name="trash" size={13} />
        </button>
      </div>
    {:else}
      <p class="hint">{t("modals.noStashesHint")}</p>
    {/each}
  </Modal>
{:else if ui.modal?.kind === "stashPreview"}
  <Modal
    title={t("modals.stashPreviewTitle", { message: ui.modal.message })}
    width="920px"
    onclose={close}
  >
    <p class="hint">{t("modals.stashPreviewHint")}</p>
    <div class="stash-preview">
      <DiffView
        findScope="stashPreview"
        diffs={stashDiff ?? []}
        loading={stashDiff === null}
        emptyText={t("modals.stashPreviewEmpty")}
      />
    </div>
  </Modal>
{:else if ui.modal?.kind === "stashPush"}
  <Modal title={t("modals.stashPushTitle")} onclose={close}>
    <p class="hint">
      {tn("modals.stashPushHint", stashCount)}
    </p>
    <label>
      <span class="lbl">{t("modals.messageOptionalLabel")}</span>
      <input
        type="text"
        placeholder={t("modals.stashMessagePlaceholder")}
        bind:value={stashMessage}
        onkeydown={(e) => e.key === "Enter" && doStashPush()}
      />
    </label>
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <button
        class="primary"
        disabled={submitting || stashCount === 0 || ui.working > 0}
        onclick={doStashPush}
      >
        {t("modals.stashAction")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "tags"}
  <Modal title={t("modals.tagsTitle")} onclose={close}>
    <div class="row">
      <input type="text" placeholder={t("modals.newTagPlaceholder")} bind:value={tagName} />
      <input type="text" placeholder={t("modals.tagMessagePlaceholder")} bind:value={tagMessage} />
      <button class="primary" disabled={!tagName.trim()} onclick={() => doCreateTag("")}>
        <Icon name="plus" size={13} />
        {t("modals.onHead")}
      </button>
    </div>
    {#each ui.tags as tag (tag.name)}
      <div class="list-row">
        <Icon name="tag" size={14} />
        <strong>{tag.name}</strong>
        <span class="grow muted">
          {tag.isAnnotated ? (tag.message ?? "") : t("modals.lightweight")} · {tag.targetId.slice(
            0,
            8,
          )}
        </span>
        <button
          class="ghost danger"
          title={t("modals.deleteTag")}
          onclick={() => deleteTag(tag.name)}
        >
          <Icon name="trash" size={13} />
        </button>
      </div>
    {:else}
      <p class="hint">{t("modals.noTags")}</p>
    {/each}
  </Modal>
{:else if ui.modal?.kind === "remotes"}
  <Modal title={t("modals.remotesTitle")} width="560px" onclose={close}>
    <div class="row">
      <input
        type="text"
        placeholder={t("modals.remoteNamePlaceholder")}
        style="max-width: 200px"
        bind:value={remoteName}
      />
      <input
        type="text"
        placeholder={t("modals.remoteUrlPlaceholder")}
        bind:value={remoteUrl}
        onkeydown={(e) => e.key === "Enter" && doAddRemote()}
      />
      <button
        class="primary"
        disabled={!remoteName.trim() || !remoteUrl.trim() || submitting}
        onclick={doAddRemote}
      >
        <Icon name="plus" size={13} />
        {t("common.add")}
      </button>
    </div>
    {#each ui.remotes as remote (remote.name)}
      {#if editingRemote === remote.name}
        <div class="list-row">
          <Icon name="globe" size={14} />
          <input
            type="text"
            style="max-width: 130px"
            bind:value={editRemoteName}
            aria-label={t("modals.remoteNameAria")}
          />
          <input
            type="text"
            class="grow"
            bind:value={editRemoteUrl}
            aria-label={t("modals.remoteUrlAria")}
            onkeydown={(e) => e.key === "Enter" && doSaveRemote(remote)}
          />
          <button class="ghost" title={t("common.cancel")} onclick={() => (editingRemote = null)}>
            <Icon name="x" size={13} />
          </button>
          <button
            class="primary"
            disabled={!editRemoteName.trim() || !editRemoteUrl.trim() || submitting}
            onclick={() => doSaveRemote(remote)}
          >
            {t("common.save")}
          </button>
        </div>
      {:else}
        <div class="list-row">
          <Icon name="globe" size={14} />
          <strong>{remote.name}</strong>
          <span class="grow muted" title={remote.url}>{remote.url}</span>
          <button
            class="ghost"
            title={t("modals.renameOrChangeUrl")}
            onclick={() => startEditRemote(remote.name, remote.url)}
          >
            <Icon name="edit" size={13} />
          </button>
          <button
            class="ghost danger"
            title={t("modals.removeRemoteTitle")}
            onclick={() => confirmRemoveRemote(remote.name)}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      {/if}
    {:else}
      <p class="hint">
        {t("modals.noRemoteHintPre")} <code>origin</code>
        {t("modals.noRemoteHintPost")}
      </p>
    {/each}
  </Modal>
{:else if ui.modal?.kind === "changeRequests"}
  <Modal
    title={crList ? t("crs.title", { label: crLabel, host: crList.host }) : (ui.repo?.name ?? "")}
    width="680px"
    onclose={close}
  >
    {#if crLoading}
      <p class="hint"><span class="spin"></span> {t("crs.loading")}</p>
    {:else if crError}
      {#if crError.code === "no_account"}
        <p class="hint">{t("crs.noAccountGeneric")}</p>
        <p class="hint muted">{crError.message}</p>
        <div class="actions">
          <button onclick={close}>{t("common.close")}</button>
          <button class="primary" onclick={crGotoSettings}>{t("crs.addAccount")}</button>
        </div>
      {:else if crError.code === "no_remote"}
        <p class="hint">{t("crs.noRemoteHint")}</p>
      {:else}
        <p class="hint">{crError.message}</p>
        <div class="actions">
          <button onclick={loadChangeRequests}>{t("crs.refresh")}</button>
        </div>
      {/if}
    {:else if crList}
      {#each crList.items as cr (cr.number)}
        <button
          class="cr-row"
          onclick={() => api.openExternal(cr.webUrl).catch(() => {})}
          title={t("crs.openInBrowser")}
        >
          <span
            class="ci-dot {cr.ciStatus}"
            role="img"
            title={t(`ci.${cr.ciStatus}`)}
            aria-label={t(`ci.${cr.ciStatus}`)}
          ></span>
          <span class="cr-num">#{cr.number}</span>
          <span class="cr-title">
            {cr.title}
            {#if cr.isDraft}<span class="cr-draft">{t("crs.draft")}</span>{/if}
          </span>
          <span class="cr-meta">
            {cr.author} · {cr.sourceBranch} → {cr.targetBranch} · {timeAgo(cr.updatedAt)}
          </span>
        </button>
      {:else}
        <p class="hint">{t("crs.none", { label: crLabel })}</p>
      {/each}
      <div class="actions">
        <button onclick={loadChangeRequests}>
          <Icon name="refresh" size={13} />
          {t("crs.refresh")}
        </button>
        <button class="ghost" onclick={openPrInBrowser}>
          <Icon name="external" size={13} />
          {t("crs.createInBrowser")}
        </button>
        <button class="primary" onclick={() => (ui.modal = { kind: "createCr" })}>
          <Icon name="plus" size={13} />
          {t("crs.createNew")}
        </button>
      </div>
    {/if}
  </Modal>
{:else if ui.modal?.kind === "createCr"}
  <Modal title={t("crs.createTitle", { label: newCrLabel })} width="560px" onclose={close}>
    {#if !newCrSource}
      <p class="hint">{t("crs.detachedHead")}</p>
    {:else if !newCrHasUpstream}
      <p class="hint">{t("crs.needUpstream")}</p>
    {:else}
      {#if newCrError}
        {#if newCrError.code === "no_account"}
          <p class="hint">{t("crs.noAccountGeneric")}</p>
          <div class="actions">
            <button onclick={close}>{t("common.close")}</button>
            <button class="primary" onclick={crGotoSettings}>{t("crs.addAccount")}</button>
          </div>
        {:else}
          <p class="hint danger-text">{newCrError.message}</p>
        {/if}
      {/if}
      {#if newCrError?.code !== "no_account"}
        <label>
          <span class="lbl">{t("crs.fieldTitle")}</span>
          <input type="text" bind:value={newCrTitle} />
        </label>
        <textarea rows="4" placeholder={t("crs.fieldDescription")} bind:value={newCrDescription}
        ></textarea>
        <div class="row">
          <label class="grow">
            <span class="lbl">{t("crs.fieldSource")}</span>
            <input type="text" value={newCrSource} disabled />
          </label>
          <label class="grow">
            <span class="lbl">{t("crs.fieldTarget")}</span>
            <input type="text" bind:value={newCrTarget} />
          </label>
        </div>
        <label class="check">
          <input type="checkbox" bind:checked={newCrDraft} />
          {t("crs.markDraft")}
        </label>
        <div class="actions">
          <button onclick={close}>{t("common.cancel")}</button>
          <button
            class="primary"
            disabled={submitting ||
              !newCrTitle.trim() ||
              !newCrTarget.trim() ||
              newCrTarget.trim() === newCrSource}
            onclick={doCreateCr}
          >
            {#if submitting}<span class="spin"></span>{/if}
            {t("crs.create")}
          </button>
        </div>
      {/if}
    {/if}
  </Modal>
{:else if ui.modal?.kind === "sparse"}
  <Modal title={t("sparse.title")} width="520px" onclose={close}>
    <p class="hint">{t("sparse.hint")}</p>
    {#if sparseLoading || !sparse}
      <p class="hint"><span class="spin"></span> {t("sparse.loading")}</p>
    {:else}
      <p class="hint">
        {sparse.enabled ? tn("sparse.active", sparse.patterns.length) : t("sparse.inactive")}
      </p>
      {#each sparse.topDirs as dir (dir)}
        <label class="check">
          <input
            type="checkbox"
            checked={sparseSelected.has(dir)}
            onchange={() => toggleSparseDir(dir)}
          />
          {dir}/
        </label>
      {:else}
        <p class="hint">{t("sparse.noDirs")}</p>
      {/each}
      {#if sparseSelected.size === 0 && sparse.topDirs.length > 0}
        <p class="hint">{t("sparse.needSelection")}</p>
      {/if}
      <div class="actions">
        {#if sparse.enabled}
          <button disabled={submitting} onclick={doSparseDisable}>
            {t("sparse.disable")}
          </button>
        {:else}
          <button onclick={close}>{t("common.cancel")}</button>
        {/if}
        <button
          class="primary"
          disabled={submitting || sparseSelected.size === 0 || sparse.topDirs.length === 0}
          onclick={doSparseApply}
        >
          {#if submitting}<span class="spin"></span>{/if}
          {t("sparse.apply")}
        </button>
      </div>
    {/if}
  </Modal>
{:else if ui.modal?.kind === "backups"}
  <Modal title={t("modals.backupsTitle")} width="620px" onclose={close}>
    <p class="hint">{t("modals.backupsHint")}</p>
    {#if backupsLoading}
      <p class="hint"><span class="spin"></span> {t("modals.backupsLoading")}</p>
    {:else}
      {#each backups as b (b.name)}
        <div class="list-row">
          <Icon name="undo" size={14} />
          <strong>{backupOpLabel(b.op)}</strong>
          <span class="grow muted wrap" title={b.targetId}>
            “{b.subject}” · {b.targetId.slice(0, 8)} · {timeAgo(b.timestamp)}
          </span>
          <button
            class="ghost"
            disabled={!!ui.busy || ui.working > 0}
            title={t("modals.restoreBackupHover")}
            onclick={() => doRestoreBackup(b)}
          >
            {t("modals.restore")}
          </button>
          <button
            class="ghost danger"
            title={t("modals.deleteBackupTitle")}
            onclick={() => doDeleteBackup(b)}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      {:else}
        <p class="hint">{t("modals.noBackups")}</p>
      {/each}
    {/if}
  </Modal>
{:else if ui.modal?.kind === "submodules"}
  <Modal title={t("modals.submodulesTitle")} width="560px" onclose={close}>
    {#if subsLoading}
      <p class="hint"><span class="spin"></span> {t("modals.submodulesLoading")}</p>
    {:else}
      {#each subs as sub (sub.path)}
        <div class="list-row">
          <Icon name="tree" size={14} />
          <strong>{sub.name}</strong>
          <span class="grow muted" title={sub.url ?? sub.path}>
            {sub.path}{sub.url ? ` · ${sub.url}` : ""}
          </span>
        </div>
      {:else}
        <p class="hint">{t("modals.noSubmodules")}</p>
      {/each}
      {#if subs.length > 0}
        <p class="hint">
          {t("modals.updateAllHintPre")} <code>git submodule update --init --recursive</code>
          {t("modals.updateAllHintPost")}
        </p>
        <div class="actions">
          <button onclick={close}>{t("common.close")}</button>
          <button class="primary" disabled={!!ui.busy} onclick={doUpdateSubmodules}>
            {#if !!ui.busy}<span class="spin"></span>{/if}
            {t("modals.updateAll")}
          </button>
        </div>
      {/if}
    {/if}
  </Modal>
{:else if ui.modal?.kind === "worktrees"}
  <WorktreesModal onclose={close} />
{:else if ui.modal?.kind === "blame" && ui.blame}
  <Modal title={t("modals.blameTitle", { file: ui.blame.file })} width="960px" onclose={close}>
    <BlameView file={ui.blame.file} lines={ui.blame.lines} />
  </Modal>
{:else if ui.modal?.kind === "switchBranch"}
  {@const target = ui.modal.target}
  {@const andThen = ui.modal.andThen}
  {@const label = switchTargetLabel(target)}
  <Modal title={t("switch.title", { name: label })} width="480px" onclose={close}>
    <p class="hint">{tn("switch.pending", stashCount, { from: switchFrom })}</p>
    <!-- "Bring along" comes first and carries the accent bar: it is the path
         git takes by itself — the choice only changes that it is made
         consciously. -->
    <button
      class="opt recommended"
      onclick={() => chooseSwitch(carryChangesAndSwitch, target, andThen)}
    >
      <span class="opt-title">{t("switch.carry", { name: label })}</span>
      <span class="opt-hint">{t("switch.carryHint")}</span>
    </button>
    <button class="opt" onclick={() => chooseSwitch(stashAndSwitch, target, andThen)}>
      <span class="opt-title">{t("switch.leave", { from: switchFrom })}</span>
      <span class="opt-hint">{t("switch.leaveHint", { from: switchFrom })}</span>
    </button>
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "squash"}
  <Modal title={t("modals.squashTitle", { n: ui.modal.count })} onclose={close}>
    <p class="hint">{t("modals.squashHint", { n: ui.modal.count })}</p>
    <textarea
      rows="4"
      placeholder={t("modals.newCommitMessagePlaceholder")}
      bind:value={squashMessage}></textarea>
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <button
        class="primary"
        disabled={!squashMessage.trim() || submitting}
        onclick={() => doSquash((ui.modal as { oldestId: string }).oldestId)}
      >
        {t("modals.squashAction")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "branchFrom"}
  <Modal title={t("modals.branchFromTitle", { id: ui.modal.commitId.slice(0, 8) })} onclose={close}>
    <input
      type="text"
      placeholder={t("modals.branchNamePlaceholder")}
      bind:value={newBranchName}
      onkeydown={(e) =>
        e.key === "Enter" && doBranchFrom((ui.modal as { commitId: string }).commitId)}
    />
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <button
        class="primary"
        disabled={!newBranchName.trim()}
        onclick={() => doBranchFrom((ui.modal as { commitId: string }).commitId)}
      >
        {t("modals.createAndCheckout")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "tagAt"}
  <Modal title={t("modals.tagAtTitle", { id: ui.modal.commitId.slice(0, 8) })} onclose={close}>
    <input type="text" placeholder={t("modals.tagNamePlaceholder")} bind:value={tagName} />
    <input
      type="text"
      placeholder={t("modals.tagMessageLightweightPlaceholder")}
      bind:value={tagMessage}
    />
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <button
        class="primary"
        disabled={!tagName.trim()}
        onclick={() => doCreateTag((ui.modal as { commitId: string }).commitId)}
      >
        {t("modals.createTag")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "cherryPickTo"}
  <Modal
    title={t("modals.cherryPickToTitle", { id: ui.modal.commitId.slice(0, 8) })}
    onclose={close}
  >
    <p class="hint">{t("modals.cherryPickToHint")}</p>
    <input type="text" placeholder={t("modals.filterBranchesPlaceholder")} bind:value={cpFilter} />
    <div class="branch-list">
      {#each cpBranches as b (b.name)}
        <button class="item ghost" disabled={ui.working > 0} onclick={() => doCherryPickTo(b.name)}>
          <Icon name="branch" size={14} />
          {b.name}
        </button>
      {:else}
        <p class="hint">{t("modals.noOtherLocalBranch")}</p>
      {/each}
    </div>
  </Modal>
{:else if ui.modal?.kind === "rebase"}
  <RebaseModal baseId={ui.modal.baseId} commits={ui.modal.commits} onclose={close} />
{:else if ui.modal?.kind === "conflictEditor"}
  <ConflictEditor file={ui.modal.file} onclose={close} />
{:else if ui.modal?.kind === "sshTofu"}
  {@const scan = ui.modal.scan}
  <Modal title={t("ssh.tofuTitle")} onclose={cancelSshTofu}>
    <p class="hint">{t("ssh.tofuIntro", { host: scan.host })}</p>
    {#if scan.changed}
      <p class="hint danger-text">{t("ssh.changedWarn")}</p>
    {/if}
    {#each scan.fingerprints as fp (fp.keyType + fp.sha256)}
      <div class="list-row">
        <strong>{fp.keyType}</strong>
        <code class="grow">{fp.sha256}</code>
      </div>
    {:else}
      <p class="hint">{t("ssh.fingerprint")}: —</p>
    {/each}
    <div class="actions">
      <button onclick={cancelSshTofu}>{t("common.cancel")}</button>
      <button
        class={scan.changed ? "danger" : "primary"}
        disabled={sshTrustBusy}
        onclick={doConfirmSshTrust}
      >
        {#if sshTrustBusy}<span class="spin"></span>{/if}
        {scan.changed ? t("ssh.trustReplace") : t("ssh.trust")}
      </button>
    </div>
  </Modal>
{:else if ui.modal?.kind === "deleteRepo"}
  {@const repoPath = ui.modal.path}
  <Modal title={t("welcome.deleteRepoTitle")} width="480px" onclose={close}>
    <p class="hint danger-text">{t("welcome.deleteRepoWarn")}</p>
    <p class="hint"><code>{repoPath}</code></p>
    <div class="actions">
      <button onclick={close}>{t("common.cancel")}</button>
      <!-- No more typing of the name: the backend command shows a native
           OS dialog (safeguard 3) and only moves to the recycle bin
           (recoverable) — a simple confirmation step is
           proportionate. -->
      <button class="danger" onclick={() => deleteRepoFromDisk(repoPath)}>
        {t("welcome.deleteRepoBtn")}
      </button>
    </div>
  </Modal>
{/if}

<style>
  .lbl {
    display: block;
    font-size: 12px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }

  .row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
  }

  .danger-text {
    color: var(--deleted);
  }

  .hint {
    color: var(--text-muted);
    font-size: 12px;
  }

  .hint code {
    font-family: var(--mono);
    color: var(--text-primary);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  /* ---- Choice cards (branch switch with uncommitted changes) ---- */

  .opt {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: flex-start;
    gap: 3px;
    width: 100%;
    padding: var(--space-3);
    text-align: left;
  }

  .opt-title {
    font-weight: 600;
  }

  .opt-hint {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 400;
    line-height: 1.45;
    /* Buttons do not inherit nowrap, but the hint MUST be allowed to wrap. */
    white-space: normal;
  }

  /* Recommendation marked as in the toast: an accent bar on the left instead of a fill —
     that way the hint text in the button stays readable. */
  .opt.recommended {
    border-left: 3px solid var(--accent);
  }

  .list-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--border);
  }

  .list-row:last-child {
    border-bottom: none;
  }

  /* ---- Change request list (PRs/MRs) ---- */

  .cr-row {
    display: grid;
    grid-template-columns: auto auto 1fr;
    grid-template-areas: "dot num title" "dot num meta";
    align-items: center;
    column-gap: var(--space-2);
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    box-shadow: none;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    padding: var(--space-2) var(--space-1);
    cursor: pointer;
  }

  .cr-row:hover {
    background: var(--bg-hover);
  }

  .ci-dot {
    grid-area: dot;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .ci-dot.success {
    background: var(--added);
  }

  .ci-dot.failed {
    background: var(--deleted);
  }

  .ci-dot.running {
    background: var(--blue);
  }

  .ci-dot.pending {
    background: var(--modified);
  }

  .ci-dot.canceled {
    background: var(--text-faint);
  }

  .ci-dot.unknown {
    background: transparent;
    border: 1.5px solid var(--border-strong);
  }

  .cr-num {
    grid-area: num;
    color: var(--text-faint);
    font-size: 12px;
  }

  .cr-title {
    grid-area: title;
    font-weight: 550;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cr-draft {
    display: inline-block;
    margin-left: 6px;
    padding: 0 6px;
    font-size: 10.5px;
    color: var(--text-muted);
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    vertical-align: middle;
  }

  .cr-meta {
    grid-area: meta;
    color: var(--text-faint);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Backup meta: wrap instead of truncating. */
  .grow.wrap {
    white-space: normal;
    overflow-wrap: anywhere;
    text-overflow: clip;
  }

  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }

  .stash-preview {
    height: 62vh;
    min-height: 240px;
  }

  .branch-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 300px;
    overflow-y: auto;
  }

  .branch-list .item {
    justify-content: flex-start;
    text-align: left;
    color: var(--text-primary);
  }
</style>
