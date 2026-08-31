<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";

  import { api, type ProviderKind, type SshKey } from "../api";
  import { i18n, setLang, t, type Lang, type MessageKey } from "../i18n.svelte";
  import {
    addProviderAccount,
    generateSshKey,
    loadSshKeys,
    refreshAccounts,
    removeProviderAccount,
    removeSshKey,
    savePrefs,
    showError,
    showInfo,
    ui,
  } from "../state.svelte";
  import Icon from "./Icon.svelte";

  // The app version, shown at the end of the App section. SECURITY.md and the
  // bug-report template both ask reporters for it, so it has to be readable
  // somewhere other than the start screen (which is gone once a repo is open).
  let appVersion = $state("");
  $effect(() => {
    getVersion()
      .then((v) => (appVersion = v))
      .catch(() => {
        // Mock/browser mode without the app plugin: the row stays hidden.
      });
  });

  // Load the git identity when entering the page (the effective, merged values).
  let cfgName = $state("");
  let cfgEmail = $state("");
  let cfgSign = $state(false);
  let cfgGlobal = $state(true);
  let loaded = $state(false);
  let saving = $state(false);
  let signChecking = $state(false);

  $effect(() => {
    if (!loaded && ui.repo) {
      loaded = true;
      Promise.all([
        api.configGet(ui.repo.path, "user.name"),
        api.configGet(ui.repo.path, "user.email"),
        api.configGet(ui.repo.path, "commit.gpgsign"),
      ])
        .then(([n, e, s]) => {
          cfgName = n ?? "";
          cfgEmail = e ?? "";
          cfgSign = s === "true";
        })
        .catch((e) => {
          showError(e);
        });
    }
  });

  async function saveIdentity() {
    if (!ui.repo || saving) return;
    saving = true;
    try {
      await api.configSet(ui.repo.path, "user.name", cfgName.trim(), cfgGlobal);
      await api.configSet(ui.repo.path, "user.email", cfgEmail.trim(), cfgGlobal);
      await api.configSet(ui.repo.path, "commit.gpgsign", cfgSign ? "true" : "false", cfgGlobal);
      showInfo(t("settings.saved"));
    } catch (e) {
      showError(e);
    } finally {
      saving = false;
    }
  }

  /** Signing preflight: try a signed, unreferenced commit object. */
  async function doCheckSigning() {
    if (!ui.repo || signChecking) return;
    signChecking = true;
    try {
      showInfo(await api.checkSigning(ui.repo.path));
    } catch (e) {
      showError(e);
    } finally {
      signChecking = false;
    }
  }

  function setTheme(t: "dark" | "light" | "system") {
    ui.theme = t;
    savePrefs();
  }

  // labelKey instead of a finished text: call t() only in the template so a
  // language change applies immediately (module/init calls would be frozen).
  const THEMES: { value: "dark" | "light" | "system"; labelKey: MessageKey; icon: string }[] = [
    { value: "dark", labelKey: "theme.dark", icon: "moon" },
    { value: "light", labelKey: "theme.light", icon: "sun" },
    { value: "system", labelKey: "theme.system", icon: "window" },
  ];

  // Language names deliberately NOT translated: every language in its own
  // spelling, so you can always find your own again.
  const LANGS: { value: Lang; label: string }[] = [
    { value: "en", label: "English" },
    { value: "de", label: "Deutsch" },
  ];

  const SCALES: { value: number; labelKey: MessageKey }[] = [
    { value: 0.9, labelKey: "a11y.sizeS" },
    { value: 1, labelKey: "a11y.sizeM" },
    { value: 1.1, labelKey: "a11y.sizeL" },
    { value: 1.25, labelKey: "a11y.sizeXl" },
  ];

  function setScale(v: number) {
    ui.uiScale = v;
    savePrefs();
  }

  // Provider accounts: load the list when entering.
  refreshAccounts();
  // SSH key manager: load the local keys (~/.ssh/*.pub) when entering.
  loadSshKeys();

  let sshName = $state("id_ed25519");
  let sshComment = $state("");
  let sshPass = $state("");
  let sshBusy = $state(false);
  let sshCopiedName = $state<string | null>(null);

  async function doGenerateSshKey() {
    if (sshBusy || !sshName.trim()) return;
    sshBusy = true;
    await generateSshKey(sshName.trim(), sshComment.trim(), sshPass);
    sshBusy = false;
    sshPass = "";
  }

  async function copyPublicKey(key: SshKey) {
    try {
      await navigator.clipboard.writeText(key.publicKey);
    } catch {
      // Clipboard failed: NO tick/success, report the error.
      showError({ code: "clipboard", message: "" });
      return;
    }
    sshCopiedName = key.name;
    showInfo(t("settings.sshCopied"));
    setTimeout(() => {
      if (sshCopiedName === key.name) sshCopiedName = null;
    }, 1500);
  }

  let accKind = $state<ProviderKind>("github");
  let accHost = $state("");
  let accToken = $state("");
  let accInsecure = $state(false);
  let accBusy = $state(false);

  async function doAddAccount() {
    if (accBusy || !accHost.trim() || !accToken.trim()) return;
    accBusy = true;
    const ok = await addProviderAccount(accHost.trim(), accKind, accToken, accInsecure);
    accBusy = false;
    if (ok) {
      accHost = "";
      accToken = "";
      accInsecure = false;
    }
  }
</script>

<div class="page">
  <div class="inner">
    <header class="head">
      <button class="ghost back" onclick={() => (ui.view = "repo")}>
        <span class="back-icon"><Icon name="chevronDown" size={14} /></span>
        {t("settings.back")}
      </button>
      <h1>{t("nav.settings")}</h1>
    </header>

    <section>
      <h2 class="section-title">{t("settings.gitIdentity")}</h2>
      <label>
        <span class="lbl">{t("settings.name")}</span>
        <input type="text" bind:value={cfgName} />
      </label>
      <label>
        <span class="lbl">{t("settings.email")}</span>
        <input type="text" bind:value={cfgEmail} />
      </label>
      <div class="sign-row">
        <label class="check">
          <input type="checkbox" bind:checked={cfgSign} />
          {t("settings.signCommits")}
        </label>
        <button class="ghost" disabled={signChecking} onclick={doCheckSigning}>
          {#if signChecking}<span class="spin"></span>{/if}
          {t("settings.testSigning")}
        </button>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={cfgGlobal} />
        {t("settings.saveGlobal")}
      </label>
      <div class="row-end">
        <button class="primary" disabled={saving} onclick={saveIdentity}>
          {#if saving}<span class="spin"></span>{/if}
          {t("common.save")}
        </button>
      </div>
    </section>

    <section>
      <h2 class="section-title">{t("settings.appSection")}</h2>
      <label>
        <span class="lbl">{t("settings.editorCmd")}</span>
        <input type="text" placeholder="code" bind:value={ui.editorCmd} onchange={savePrefs} />
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={ui.autoFetch} onchange={savePrefs} />
        {t("settings.autoFetch")}
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={ui.pruneOnPull} onchange={savePrefs} />
        {t("settings.pruneOnPull")}
      </label>
      <p class="hint-text">{t("settings.pruneOnPullHint")}</p>
      <label>
        <span class="lbl">{t("settings.toastDuration")}</span>
        <select bind:value={ui.toastDuration} onchange={savePrefs}>
          <option value={2}>{t("settings.toast2s")}</option>
          <option value={4}>{t("settings.toast4s")}</option>
          <option value={8}>{t("settings.toast8s")}</option>
          <option value={0}>{t("settings.toastOff")}</option>
        </select>
      </label>
      {#if appVersion}
        <p class="hint-text">{t("settings.version", { version: appVersion })}</p>
      {/if}
    </section>

    <section>
      <h2 class="section-title">{t("settings.accounts")}</h2>
      <p class="hint-text">{t("settings.accountsHint")}</p>
      {#each ui.accounts as acc (acc.host)}
        <div class="acc-row">
          <Icon name="globe" size={14} />
          <strong>{acc.host}</strong>
          <span class="acc-kind">
            {acc.kind === "github"
              ? "GitHub"
              : acc.kind === "gitlab"
                ? "GitLab"
                : "Gitea / Forgejo"}
          </span>
          <span class="acc-user">
            @{acc.username}{acc.insecureTls ? ` · ${t("settings.insecureBadge")}` : ""}
          </span>
          <button
            class="ghost danger"
            title={t("common.delete")}
            onclick={() => removeProviderAccount(acc.host)}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      {:else}
        <p class="hint-text">{t("settings.noAccounts")}</p>
      {/each}
      <div class="acc-add">
        <select bind:value={accKind}>
          <option value="github">GitHub</option>
          <option value="gitlab">GitLab</option>
          <option value="gitea">Gitea / Forgejo</option>
        </select>
        <input type="text" placeholder={t("settings.hostPlaceholder")} bind:value={accHost} />
      </div>
      <input
        type="password"
        placeholder={t("settings.tokenPlaceholder")}
        bind:value={accToken}
        onkeydown={(e) => e.key === "Enter" && doAddAccount()}
      />
      <p class="hint-text">{t("settings.tokenHint")}</p>
      {#if accKind === "gitlab" || accKind === "gitea" || accInsecure}
        <label class="check">
          <input type="checkbox" bind:checked={accInsecure} />
          {t("settings.insecureTls")}
        </label>
      {/if}
      <div class="row-end">
        <button
          class="primary"
          disabled={accBusy || !accHost.trim() || !accToken.trim()}
          onclick={doAddAccount}
        >
          {#if accBusy}<span class="spin"></span>{/if}
          {t("settings.connect")}
        </button>
      </div>
    </section>

    <section>
      <h2 class="section-title">{t("settings.sshSection")}</h2>
      <p class="hint-text">{t("settings.sshHint")}</p>
      {#each ui.sshKeys as key (key.name)}
        <div class="acc-row">
          <Icon name="file" size={14} />
          <strong>{key.name}</strong>
          <span class="acc-kind">{key.keyType}</span>
          <span class="acc-user" title={key.fingerprint}>
            {key.comment}{key.comment ? " · " : ""}{key.fingerprint}
          </span>
          <button class="ghost" title={t("settings.sshCopyPub")} onclick={() => copyPublicKey(key)}>
            <Icon name={sshCopiedName === key.name ? "check" : "copy"} size={13} />
          </button>
          <button
            class="ghost danger"
            title={t("settings.sshDelete")}
            onclick={() => removeSshKey(key.name)}
          >
            <Icon name="trash" size={13} />
          </button>
        </div>
      {:else}
        <p class="hint-text">{t("settings.sshNoKeys")}</p>
      {/each}
      <div class="acc-add">
        <input type="text" placeholder={t("settings.sshName")} bind:value={sshName} />
        <input type="text" placeholder={t("settings.sshComment")} bind:value={sshComment} />
      </div>
      <input
        type="password"
        placeholder={t("settings.sshPassphrase")}
        bind:value={sshPass}
        onkeydown={(e) => e.key === "Enter" && doGenerateSshKey()}
      />
      <div class="row-end">
        <button class="primary" disabled={sshBusy || !sshName.trim()} onclick={doGenerateSshKey}>
          {#if sshBusy}<span class="spin"></span>{/if}
          {t("settings.sshGenerate")}
        </button>
      </div>
    </section>

    <section>
      <h2 class="section-title">{t("theme.title")}</h2>
      <div class="themes">
        {#each THEMES as opt (opt.value)}
          <button
            class="theme-opt"
            class:active={ui.theme === opt.value}
            onclick={() => setTheme(opt.value)}
          >
            <Icon name={opt.icon} size={15} />
            {t(opt.labelKey)}
            {#if ui.theme === opt.value}
              <span class="mark"><Icon name="check" size={12} /></span>
            {/if}
          </button>
        {/each}
      </div>
    </section>

    <section>
      <h2 class="section-title">{t("settings.language")}</h2>
      <p class="hint-text">{t("settings.languageHint")}</p>
      <div class="themes">
        {#each LANGS as opt (opt.value)}
          <button
            class="theme-opt"
            class:active={i18n.lang === opt.value}
            onclick={() => setLang(opt.value)}
          >
            <Icon name="globe" size={15} />
            {opt.label}
            {#if i18n.lang === opt.value}
              <span class="mark"><Icon name="check" size={12} /></span>
            {/if}
          </button>
        {/each}
      </div>
    </section>

    <section>
      <h2 class="section-title">{t("settings.a11ySection")}</h2>
      <span class="lbl">{t("settings.fontSize")}</span>
      <div class="themes">
        {#each SCALES as opt (opt.value)}
          <button
            class="theme-opt"
            class:active={ui.uiScale === opt.value}
            onclick={() => setScale(opt.value)}
          >
            {t(opt.labelKey)}
            {#if ui.uiScale === opt.value}
              <span class="mark"><Icon name="check" size={12} /></span>
            {/if}
          </button>
        {/each}
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={ui.reduceMotion} onchange={savePrefs} />
        {t("settings.reduceMotion")}
      </label>
      <p class="hint-text">{t("settings.reduceMotionHint")}</p>
      <label class="check">
        <input type="checkbox" bind:checked={ui.highContrast} onchange={savePrefs} />
        {t("settings.highContrast")}
      </label>
    </section>
  </div>
</div>

<style>
  .page {
    height: 100%;
    overflow-y: auto;
    background: var(--bg-app);
  }

  .inner {
    max-width: 640px;
    margin: 0 auto;
    padding: var(--space-5) var(--space-4) var(--space-6);
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  h1 {
    font-family: var(--display);
    font-size: 20px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .back-icon {
    display: inline-flex;
    transform: rotate(90deg);
  }

  section {
    background: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .hint-text {
    font-size: 12px;
    color: var(--text-muted);
    margin-bottom: var(--space-2);
  }

  .acc-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--border);
  }

  .acc-row:last-of-type {
    border-bottom: none;
  }

  .acc-kind {
    font-size: 10.5px;
    color: var(--text-muted);
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    padding: 0 6px;
  }

  .acc-user {
    flex: 1;
    min-width: 0;
    color: var(--text-faint);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .acc-add {
    display: flex;
    gap: var(--space-2);
  }

  .acc-add select {
    flex: 0 0 auto;
    width: auto;
    min-width: 110px;
  }

  .acc-add input {
    flex: 1;
    min-width: 0;
  }

  .lbl {
    display: block;
    font-size: 12px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }

  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-primary);
  }

  .sign-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .row-end {
    display: flex;
    justify-content: flex-end;
  }

  .themes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-2);
  }

  .theme-opt {
    justify-content: flex-start;
    padding: 8px 12px;
  }

  .theme-opt.active {
    border-color: var(--accent-dim);
  }

  .mark {
    margin-left: auto;
    display: inline-flex;
    color: var(--accent);
  }
</style>
