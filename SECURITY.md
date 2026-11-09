# Security Policy

## Supported Versions

Only the latest release is actively supported with security updates.

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |
| < latest | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue
in Nuphus, please report it responsibly.

### Report Channels

- **GitHub Security Advisory**: [https://github.com/mrpulor-gh/nuphus/security/advisories/new](https://github.com/mrpulor-gh/nuphus/security/advisories/new)

### Response Commitment

- **Initial Response**: Within 48 hours (business days)
- **Status Update**: At least every 5 business days until resolution
- **Disclosure**: We aim to publish advisories within 90 days, or earlier if
  coordinated with the reporter

### What to Include

When reporting, please provide as much of the following as possible:

- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Potential impact
- Suggested mitigations or fixes (if any)

### Scope

The following are in scope for our security program:

- The Nuphus desktop application (Tauri shell, Rust backend)
- The Nuphus web frontend (React 18 + TypeScript)
- CLI tooling shipped with Nuphus
- Plugin execution sandbox and permission model
- The mobile companion server (`src-tauri/src/mobile_server.rs`) and its
  authentication, pairing, and network exposure surface
- The relay server (`relay-server/`) and the relay client tunnel

The following are generally out of scope:

- Issues in dependencies unless they directly affect Nuphus in a novel way
- Theoretical attacks requiring physical access to the user's machine
- Social engineering attacks
- Denial of service via resource exhaustion on a local machine
- Issues in user-authored plugins or custom workflows

## Threat Model

This section documents the attack surface Nuphus intentionally exposes, and
the controls that guard it. It is written so that a reporter can tell the
difference between a design tradeoff and a bug.

### Mobile companion server

The mobile server is an HTTP + WebSocket server (`axum`) that lets a phone act
as a second screen for the same agent. It is **off by default** and only runs
when the user enables it in Settings.

**Exposure by design.** When enabled, the server binds `0.0.0.0` (see
`mobile_server.rs` module docs) so a phone on the same network can reach it.
Any device on the same LAN can therefore attempt a connection, and the channel
is plain HTTP — there is no TLS. This is a deliberate tradeoff for zero-config
local discovery; the mitigations below are what stand between that exposure and
a compromised phone session.

**Controls that guard it:**

- **Default-off.** The server does not start unless the user turns it on, and it
  auto-restores only when a prior `enabled=true` was persisted.
- **Token auth on the business endpoints.** 29 of the 37 routes require
  `X-Mobile-Token` (header) or `?token=` (query) via `token_valid`. The query
  form exists only because the browser WebSocket API cannot set custom headers;
  both forms are equally accepted.
- **Pairing password required to obtain a token.** A token is only handed out
  by `POST /pair` after the desktop-set pairing password is verified. The
  password is never stored in plaintext — only a salted SHA-256 digest
  (`{salt}:{hex}`, `hash_password`), with a fresh salt per setup.
- **Brute-force throttling.** `POST /pair` locks out after 5 consecutive failed
  attempts for 60 seconds.
- **Unauthenticated surface is enumerated, not assumed.** Exactly eight routes
  are reachable without a token:
  - `GET /health` — a static status object;
  - `POST /pair` — the pairing endpoint. It *cannot* require a token, since its
    whole purpose is to issue one; it is instead guarded by the pairing password
    plus the lockout above;
  - six static-asset routes: `GET /`, `GET /plugins/*rest`, and
    `GET /plugins-shared/{tokens,base,theme}.css` + `bridge.js`.

  The remaining 29 routes — including `/file` and `/ws` — all run `token_valid`.
  Unauthenticated static-asset serving is expected; this list exists so a
  reporter does not have to guess where the boundary is.
- **CORS is an allowlist.** `Access-Control-Allow-Origin` is echoed only for
  three classes of origin: the relay tunnel's public origin, `localhost` /
  `127.0.0.1` (development), and a private-network host whose port equals the
  mobile server's own listening port (covers LAN pages that later fall back to
  the relay). Any other origin receives no ACAO header, so a browser cannot be
  used to read responses from the server or to trigger the pairing lockout
  remotely. Token auth does not depend on CORS.
- **File access is constrained.** `GET /file` **requires an absolute path** —
  relative paths are rejected with 400, because resolving them against the
  process working directory would escape the user's expectation and is a
  traversal vector. It also rejects any `..` parent component, requires a
  regular file, enforces a 30 MB per-file cap, and serves only an
  image-extension allowlist (BMP is read and re-encoded to PNG for iOS WebView
  compatibility). No directory listing, no globbing, no arbitrary file types.

**Known limitations (accepted in the current design):**

- **No transport encryption.** Because the channel is plain HTTP, an attacker
  who can sniff the LAN (e.g. hostile WiFi) can observe the token and session
  content in transit. Do not use the mobile server on untrusted networks. For
  access beyond the local network, use a self-hosted relay server; the official
  relay is temporarily unavailable.
- **Pairing password hashing is a single-round salted SHA-256**, not a
  memory-hard KDF. It is adequate against casual inspection of the config file
  but is not designed to resist offline GPU cracking of a weak password. Choose
  a strong pairing password.
- **Token lifetime is process/config-scoped**, not per-session or expiring. A
  leaked token remains valid until the user regenerates it.

### Relay server

A user-deployed relay is a routing component, not storage: it validates
device/caller identity and forwards traffic; it does not persist, cache, or
inspect session content. Protocol and deployment notes live in
[`relay-server/README.md`](relay-server/README.md). The relay tunnel carries
its own `X-Relay-Token` and multi-tenant `X-Tunnel-Device` attribution alongside
the mobile token. Anything that would change the "no on-disk storage" property
is in scope as a vulnerability.

### Secret storage

Model provider API keys are written to `config.toml`. On **Windows** they are
encrypted with DPAPI bound to the current user and stored as `enc:v1:<base64>`
(`src/cookies/vault.rs`, `encrypt_secret`). On **macOS and Linux** the same
function intentionally falls back to storing the key in plaintext, relying on
filesystem permissions. A plaintext key on those platforms is a known,
documented gap — please report exposures, but treat plaintext storage on
non-Windows as documented behavior rather than a new vulnerability.

### Safe Harbor

We will not pursue legal action against researchers who:

- Make a good faith effort to avoid privacy violations, data destruction, or
  service disruption
- Report vulnerabilities promptly through the channels listed above
- Provide a reasonable time for us to address the issue before any public
  disclosure

## Security Best Practices for Users

- Always run the latest version of Nuphus
- Review plugin permissions before installation
- Keep your operating system and dependencies up to date
- Do not run Nuphus with elevated privileges unless absolutely necessary
