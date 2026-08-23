# Nuphus — Local-First AI Agent

[![Apache-2.0 License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-orange)](https://rustup.rs/)
[![Tauri](https://img.shields.io/badge/Tauri-v2-ffc131)](https://v2.tauri.app)
[![React](https://img.shields.io/badge/React-18-61dafb)](https://react.dev)

**English** | [中文](README.md)

> **Version**: 0.1.x · **Status**: Alpha (under active development) · **Platforms**: Windows / macOS / Linux
> **Tech Stack**: Tauri v2 · Rust · React 18 · TypeScript

<p align="center">
  <img src="docs/readme-hero/preview.png" alt="Nuphus desktop & mobile" width="100%">
</p>

**Nuphus is an AI Agent that runs on your computer — local, private, and with real desktop execution power. Your phone is its second screen.**

It reads the screen, drives the mouse and keyboard, controls windows, reads and writes files, and orchestrates the browser — turning LLM reasoning into real automation. Data stays on your machine, the model is your choice, and the Agent gets things done for you.

Every session you start on the desktop is synced to your phone in real time — the same memory, the same Agent, the same conversation, continuing wherever you are.

---

## Why Nuphus

Existing agent products all work within their own boundaries:

| Category | Examples | Boundary |
|----------|----------|----------|
| Coding agents | Cursor, Cline | Can't leave the IDE |
| Chat agents | OpenClaw | Remote control inside a chat app |
| Cloud agents | Codex Computer Use | Runs in someone else's VM |

Nuphus breaks these boundaries. It isn't just a tool that "helps you write code" — it's an **Agent that does your daily work for you**, and it follows you wherever you go.

### Design philosophy

The fundamental tension of an agent: reasoning needs intelligence, but repeated execution needs determinism. Making an LLM spend tokens to solve 85%-identical tasks every time is a double waste of compute and time.

Nuphus's answer — let the LLM reason once, compile it into a workflow, then execute repeatedly with **zero tokens**:

```
User intent → LLM reasons & explores once (compile) → deterministic workflow → ChatAgent smart decision points → engine repeats
```

Compilation is a one-time cost that buys **deterministic, repeatable, near-zero marginal cost** automation.

> **Typical scenario**: "Back up my project folder to the desktop every day at 6 PM" → Nuphus compiles it into a workflow → it runs automatically every day after that, zero tokens, zero conversation. When you're out and about, just glance at the execution status on your phone.

### Serial reasoning, parallel execution

Nuphus's core engine is built in Rust; tool calls complete in milliseconds. The latency bottleneck is model inference, not the engine.

Agent decisions are themselves causal chains — each step depends on the result of the previous one, and desktop system operations should follow serial logic. Nuphus's parallelism lives in the execution layer: through explicit instruction passing, it can simultaneously open Cline to fix a bug, Claude Code to write tests, and the browser to look up docs — Nuphus can orchestrate as much parallelism as each external agent has. External agents have their own context understanding; Nuphus sits on top, verifying with screenshots, reading the output, and consolidating decisions.

**What is serial is thinking; what is parallel is execution.**

---

## Core Highlights

### Real desktop execution

Nuphus installs directly on your OS with native-grade screen awareness and input control:

- **Drive any GUI** — window + OCR + mouse/keyboard; automate any desktop app or webpage without needing their API
- **Built-in browser** — a programmable browser engine; web automation, data collection, and form interaction all happen on your machine
- **Coding & project analysis** — project analysis, code generation, debugging diagnostics, and file operations with deep understanding of project context
- **Multi-agent orchestration** — orchestrate external agents like Cline and Claude Code in parallel, consolidating their decisions

### Dual-device real-time sync: the desktop works, the phone controls

Nuphus separates "execution" from "control": **the desktop is the Agent's hands; the phone is your remote control.**

- **Same session, synced on both ends** — the phone connects to the very same desktop Agent: messages go through the same session entry point, and both devices share history, memory, and state — continue the conversation from either end
- **Real-time event stream** — every step of the Agent (thinking, tool calls, execution results) is pushed to the phone over WebSocket in real time; watch the computer work from your couch
- **Workflow remote control** — pause, resume, or stop a running workflow right from the phone, taking over control at any time
- **Execution trace replay** — review the Agent's full execution trace on the phone; every step is transparent and auditable
- **Remote access is free** — automatic direct connection on LAN (zero config); away from home, remote access goes through a relay server that stores nothing and forwards no content — identity verification and routing only

Dual-channel auto-switching: on the same Wi-Fi the phone connects directly to the desktop (fast, free); away from the LAN it automatically switches to the relay (stable, reliable), and switches back to direct when you return.

### Local-first, privacy by default

- **Data stays local** — conversations, memory, and plugins are all stored on your machine
- **Local AI engine** — PP-OCRv4 (OCR) and Candle (semantic search) run entirely locally; everyday recognition costs zero API calls
- **4-layer security** — permission switches → human-in-the-loop → injection detection → circuit-breaker protection
- **Model freedom** — unified access to major vendors including OpenAI / Anthropic / DeepSeek / Qwen / Zhipu; switch anytime

### More capabilities

| Capability | Description |
|------------|-------------|
| **Memory system** | Cross-session experience accumulation; SQLite persistence + FTS5 + vector semantic retrieval — the more you use it, the better it knows you |
| **Zero-compile extensions** | Knowledge bases, skills, workflows, and ui-maps are all plain-text files; drop them into `plugin/` and they take effect |
| **Three-layer vision** | Built-in OCR (zero API cost) → configurable vision model (complex scenes) → user-guided vision (most flexible fallback) |

---

## Installation

### One-click npm install (recommended)

One command installs everything and automatically matches your platform's binary (Windows x64 / macOS arm64 / Linux x64). **No installer download, no Node.js / Rust environment needed**:

```bash
# Global install (provides the `nuphus` command)
npm install -g @nuphus/nuphus-desktop

# Or try it without installing (doesn't write to global)
npx @nuphus/nuphus-desktop
```

After installation, type `nuphus` in your terminal to launch.

> The first install is large (the desktop app bundles local OCR / speech models), so please be patient.

### Download the installer

For users unfamiliar with the command line — **no terminal, no Node.js / Rust environment needed**:

1. Download the installer for your platform from [GitHub Releases](https://github.com/mrpulor-gh/nuphus/releases):
   - **Windows**: `.exe` (NSIS installer, per-user install, **no administrator rights needed**)
   - **macOS**: `.dmg`
   - **Linux**: `.deb` / `.AppImage`
2. Double-click the installer to complete the install (on Windows a **Nuphus** desktop shortcut is created)
3. Double-click the shortcut to launch

### Build from source (developers)

**Prerequisites:**

| Tool | Version | Purpose |
|------|---------|---------|
| [Rust](https://rustup.rs/) | ≥ 1.78 | Compile the core engine |
| [Node.js](https://nodejs.org/) | ≥ 18 | Tauri frontend build |
| Tauri CLI | `cargo install tauri-cli --version "^2"` | Desktop app development |

```bash
git clone https://github.com/mrpulor-gh/nuphus.git
cd nuphus

# Install dependencies (root Tauri CLI + frontend dependencies)
npm install
cd frontend && npm install && cd ..

# Launch the desktop app (dev mode: compiles Rust + starts the frontend automatically)
npx tauri dev

# Production build (outputs installers to src-tauri/target/release/bundle/)
npx tauri build
```

> The root `npm run dev` / `npm run build` are frontend-only commands; for the desktop app use `npx tauri dev` / `npx tauri build`.

### First-time setup

First launch shows a 2-step onboarding (applies to all install methods above):

1. **Choose a model provider** — pick from the preset templates (OpenAI / Anthropic / DeepSeek / Qwen / Zhipu, etc.)
2. **Enter your API Key** — press Enter to submit and you're done

> Environment variables are also supported for config-free launch: `QWEN_API_KEY="sk-xxx" npx tauri dev`

### Connect your phone

1. Enable the mobile service in the desktop "Phone" settings page (default port 18772)
2. Open the pairing page in your phone's browser and enter the pairing password to bind
3. Add to home screen (PWA) and use it like an app

Auto LAN direct-connect on the same Wi-Fi; auto relay remote channel when away from the LAN — all zero-config.

---

## Architecture Overview

Nuphus uses a six-layer architecture, bottom-up, with security woven through every layer:

```
┌──────────────────────────────┐
│ Tauri shell                  │  ← frontend UI + OS-level abilities (notifications, tray, hotkeys)
├──────────────────────────────┤
│ Runtime                      │  ← unified main loop, 3-mode routing (Free / Plan / Workflow)
├──────────────────────────────┤
│ Agent                        │  ← Leader decision / ExecAgent execution / WorkflowAgent design
├──────────────────────────────┤
│ Transport                    │  ← multi-provider abstraction (major AI vendors unified)
├──────────────────────────────┤
│ Tools / Memory / Workflow    │  ← execution infrastructure
├──────────────────────────────┤
│ Security / Permissions       │  ← security chain across all layers (injection detection / permission tiers / review)
└──────────────────────────────┘
```

**Dual-device sync architecture**:

```
┌───────────────┐   WebSocket live event stream   ┌─────────────────┐
│ Phone PWA     │ ←────────────────────────────→  │ Desktop mobile  │
│ (chat/remote) │   POST /message shared entry     │ server (18772) │
└───────────────┘                                  └────────┬────────┘
        ↕ LAN direct (auto on same Wi-Fi)                  │ shared session/memory/state
┌───────────────┐                                  ┌────────┴────────┐
│ Relay server  │ ←────── remote channel (free) ──→ │ Nuphus desktop  │
│  (no storage) │                                  │  Agent engine   │
└───────────────┘                                  └─────────────────┘
```

Phone messages go through `submit_user_message(source="mobile")`, sharing the same `leader_agent` / busy lock / dedup logic as the desktop — both devices are **two front-ends for the same Agent**, not two independent systems. On disconnect it reconnects with exponential backoff, and after reconnecting it re-fetches history to fill the gap — no messages are lost.

**Data flow**: user input → Tauri event → Runtime routing → Leader decision → `task_dispatch` → ExecAgent execution → result returns → frontend display (synced on desktop and phone)

---

## Configuration

Nuphus uses a TOML config file, searched by priority in `src/config/mod.rs::load_registry`:

| # | Path | Use case |
|---|------|----------|
| 1 | `<exe_dir>/config.toml` | Portable / green deployment |
| 2 | `./config.toml` | Development |
| 3 | `~/.config/nuphus/config.toml` | Linux/macOS user-level |
| 4 | `~/.nuphus/config.toml` | Backward compatibility |
| 5 | `<AppData>/nuphus/config.toml` | Windows desktop, auto-generated on first launch |

> After onboarding you can edit settings in the "Settings → Models" panel (currently stored in plain text in the local config.toml; encrypted storage is on the roadmap).

---

## Design Principles

1. **Local-first** — data stays on your machine by default; the cloud only steps in when you choose
2. **Minimal mental model** — every feature stays as simple as possible, avoiding over-abstraction
3. **Determinism first** — compile to a workflow instead of re-reasoning; reuse instead of rewriting
4. **Long-Term First** — prefer solutions consistent with the existing architecture and maintainable over time
5. **Restraint over accumulation** — every new feature must prove itself irreplaceable
6. **Closed-loop design** — every feature module forms a complete loop from input to output

---

## How to Contribute

Nuphus is a community-driven open-source project. Besides code, you can help grow the ecosystem in these ways:

### Contribute plugins (opening soon)

| Plugin type | Description | Examples |
|-------------|-------------|----------|
| **ui-maps** | Interface layout descriptions for any software (button positions, window recognition signatures) | Photoshop export panel, enterprise ERP layouts |
| **workflows** | Reusable workflow templates | "Daily project folder backup", "Batch image compression" |
| **skills** | Domain methodologies and operating guides | "Frontend UI design standards", "Code patterns for specific frameworks" |
| **knowledge** | Project domain knowledge documents | API references, configuration notes, architecture docs |

All plugins are plain-text files (.md / .json) — drop them into the matching directory under `plugin/` and Nuphus loads them.

### Join the discussion

- GitHub Issues: bug reports and feature requests
- GitHub Discussions: usage questions, experience sharing, plugin recommendations

---

## License

Copyright © 2026 Nuphus Team · Apache License 2.0
