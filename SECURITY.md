# Security

terra-git is a Git client. It accesses the OS keychain, starts system
`git` as a sidecar and registers itself as a `git-credential-helper`
— bugs in these paths can expose credentials. I take reports about them
seriously and will answer them.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting channel:

> **Security → Report a vulnerability** at
> <https://github.com/Rynos-PCP/Terra-Git/security/advisories/new>

The channel is private end to end, does not need an e-mail address from you,
and on confirmation creates an advisory with a CVE request right away.

Helpful in a report:

- affected version (Settings → App, or the start-screen footer) and operating system
- version of system `git` (`git --version`) — many paths go through it
- the shortest sequence of steps that triggers the problem
- what an attacker can achieve with it

**Time frame:** acknowledgement within 72 hours, a first assessment within
7 days. The project is maintained in spare time — for a confirmed
vulnerability I will give you a realistic date rather than promise one I
cannot keep.

## Supported versions

| Version | Security updates |
|---|---|
| latest release | yes |
| older releases | no — please update |

There is currently **no auto-update**. Updates have to be installed manually;
watch the [releases](https://github.com/Rynos-PCP/Terra-Git/releases) for that.

## Areas that matter most

If you are looking deliberately, these are the places where a finding weighs the most:

- **Credential bridge** — `apps/desktop/src-tauri/src/credential_helper.rs`,
  `crates/tg-auth/`. Tokens live exclusively in the OS keychain (Windows
  Credential Manager / macOS Keychain / Secret Service) and must never end up in
  files, logs or process arguments.
- **git sidecar** — `crates/tg-git-engine/src/sidecar.rs`. Every argument list
  must be injection-proof (`--` separator, literal pathspecs); `ext::` remotes
  are blocked globally.
- **Tauri edge** — `apps/desktop/src-tauri/src/commands.rs`,
  `capabilities/default.json`, the CSP in `tauri.conf.json`. Capabilities are
  deny-by-default; any way to start arbitrary processes from the WebView
  or to write outside the opened repository is a finding.
- **Provider clients** — `crates/tg-providers/`. TLS verification may only be
  disabled per host and explicitly.

## What is **not** a security issue

- The installers are **not signed**. Windows SmartScreen and macOS
  Gatekeeper therefore warn about an “unknown publisher”. This is
  known and noted in the release notes — code-signing certificates are
  still pending.
- The known race in hunk staging: reading the file, rendering its hunks and
  applying your choice are three separate steps, so a program that rewrites the
  file in between can make the hunk that is staged or discarded differ from the
  one that was displayed. It is listed under Known limitations in
  [CHANGELOG.md](CHANGELOG.md). New insights on it are of course still welcome.
- Attacks that already require full access to the user account: anyone who
  can read the keychain does not need terra-git.
