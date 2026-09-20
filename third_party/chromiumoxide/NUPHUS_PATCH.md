# Nuphus Chromiumoxide Patch

This directory contains a repository-local patch of `chromiumoxide` 0.9.1.
The upstream source is distributed under `MIT OR Apache-2.0`; both upstream
license files are retained beside this document.

## Why it is vendored

Chromiumoxide normally sends `Runtime.enable` for every attached page and
keeps the Runtime domain enabled. Chromium serializes console arguments while
that domain is active, which lets a page observe CDP through an
`Error.prepareStackTrace` hook. Nuphus drives a user-visible browser, so the
connection uses an opt-in on-demand execution-context mode instead.

The default remains `RuntimeExecutionMode::Persistent`, preserving the
published crate's behavior for every caller that does not explicitly opt in.
Nuphus selects `RuntimeExecutionMode::OnDemand` for managed headed, managed
headless, same-profile reconnect, and external CDP attachment paths.

## Local changes

- `src/handler/mod.rs` and `src/handler/target.rs` propagate the typed runtime
  execution mode from `HandlerConfig` to each target and page.
- `src/handler/frame.rs` omits `Runtime.enable` in on-demand mode and clears
  cached context identifiers after document navigation.
- `src/handler/domworld.rs` adds an ID-only context setter for contexts found
  on demand without an accompanying unique context identifier.
- `src/handler/page.rs` obtains isolated-world identifiers directly from
  `Page.createIsolatedWorld`. Main-world identifiers are obtained with a
  randomized temporary `Runtime.addBinding` handshake, bounded retries, and
  cleanup of the binding, script, listener, and injected globals.
- `src/page.rs` leaves expression evaluation without an explicit context in
  on-demand mode. Chrome then evaluates in the inspected page's default world.
- `src/browser/config.rs` treats `disable_default_args()` literally and does
  not add `--disable-extensions` behind the caller's back.
- Unit tests cover compatibility mode, on-demand initialization, and context
  invalidation after navigation.

No user-agent, WebGL, GPU, hardware, timing, or interaction values are
spoofed. A failed on-demand lookup returns an error and never falls back to
`Runtime.enable`.

## References

- https://github.com/puppeteer/puppeteer/tree/main/packages/puppeteer-core
- https://github.com/rebrowser/rebrowser-patches
- https://github.com/liqi0816/chrome-devtools-mcp-rebrowser

The binding handshake follows the Rebrowser `addBinding` design, adapted to
Chromiumoxide's Rust target state machine and typed CDP commands.

## Upgrade checklist

1. Replace the directory with the desired upstream Chromiumoxide release and
   retain its original license and release metadata.
2. Reapply the changes listed above; search the new source for every
   `Runtime.enable` and every target/session initialization path.
3. Confirm `HandlerConfig::default()` still selects persistent mode.
4. Run the vendored unit tests and the complete workspace Rust gates.
5. Run the ignored local probe in both headed and headless Chrome.
6. Run the ignored `deviceandbrowserinfo.com` acceptance test and confirm
   `isBot`, `isAutomatedWithCDP`, and `hasInconsistentTimingResolution` are all
   false.
7. Exercise external CDP attachment, navigation, expression/function
   evaluation, snapshots, frames, workers, popups, uploads, downloads,
   cookies, and reconnect behavior on each supported operating system.
