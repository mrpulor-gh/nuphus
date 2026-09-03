# Nuphus — Local-First AI Companion for Daily Coding, Office Work & Automation Workflows

[![Apache-2.0 License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.95%2B-orange)](https://rustup.rs/)
[![Tauri](https://img.shields.io/badge/Tauri-v2-ffc131)](https://v2.tauri.app)
[![React](https://img.shields.io/badge/React-18-61dafb)](https://react.dev)
[![npm version](https://img.shields.io/npm/v/@nuphus/nuphus-desktop.svg)](https://www.npmjs.com/package/@nuphus/nuphus-desktop) [![npm downloads](https://img.shields.io/npm/dm/@nuphus/nuphus-desktop.svg)](https://www.npmjs.com/package/@nuphus/nuphus-desktop) [![CI](https://github.com/mrpulor-gh/nuphus/actions/workflows/ci.yml/badge.svg)](https://github.com/mrpulor-gh/nuphus/actions/workflows/ci.yml)

**English** | [中文](README.md)

> **Version**: 0.2.5 · **Status**: Alpha (under active development) · **Platforms**: Windows / macOS / Linux
> **Tech Stack**: Tauri v2 · Rust · React 18 · TypeScript

<p align="center">
  <img src="docs/readme-hero/preview.png" alt="Nuphus desktop & mobile" width="100%">
</p>

**Nuphus is an AI Agent that runs on your computer — local, private, and with real desktop execution power. Your phone is its second screen.**

It reads the screen, drives the mouse and keyboard, controls windows, reads and writes files, and orchestrates the browser — turning LLM reasoning into real automation. Data stays on your machine, the model is your choice, and the Agent gets things done for you. Every session you start on the desktop is synced to your phone in real time — the same memory, the same Agent, the same conversation, continuing wherever you are.

---

## Design Philosophy

We don't chase flashy tricks — we value practical capability. Minimalist pragmatism: features appear on demand; no bloated shells, no repeated exploration, no wasted effort. Everything must land in daily work.

### Leader: Serial thinking, parallel execution

Nuphus's core engine is built in Rust; tool calls complete in milliseconds, so the latency bottleneck is model inference, not the engine. Agent decisions are themselves causal chains — each step depends on the result of the previous one, and desktop system operations should follow serial logic.

- **Global understanding, task ownership** — the Leader parses the goal, plans the path, monitors progress, and consolidates delivery
- **Internal orchestration (serial)** — dispatches built-in GoalType ExecAgents (project analysis / code generation / debugging / file operations, etc.) along the causal chain
- **External orchestration (parallel)** — through explicit interaction instructions, it can open Cline to fix a bug, Claude Code to write tests, and the browser to look up docs simultaneously — each external agent has its own context; Nuphus sits on top, verifying with screenshots, reading output, and consolidating decisions
- **Context passing** — explicit instructions give external agents task context, reducing repeated instructions and re-exploration

**What is serial is thinking; what is parallel is execution.**

### Workflow: Turning one exploration into deterministic execution

Making an LLM spend tokens to solve 85%-identical tasks every time is a double waste of compute and time. Nuphus's answer — **let the LLM reason once, compile it into a workflow, then execute repeatedly with near-zero tokens**:

```
Natural language interaction → Agent co-explores (understand UI/flow/params) → solidify parameters step by step → compile into a deterministic execution sequence → engine repeats
```

This is not scheduled tasks — it is **turning one intelligent exploration into repeatable deterministic execution**:

- **Natural language exploration** — interact with the Agent in natural language: grading exam papers, browser automation, desktop software operations... the Agent understands intent, recognizes the UI, and operates frame by frame
- **Parameter solidification** — after co-exploring, solidify each step's locating, operation, and exception handling into parameters (parameters are the contract, all backed by UI evidence)
- **Deterministic execution** — the compiled sequence repeats with near-zero tokens, no longer relying on the LLM to re-reason every time
- **Guardian repair** — at runtime the Agent stands guard: on errors, it fixes, records, and optimizes according to the design intent and goal, rather than blindly retrying

One compile-time investment buys **deterministic, repeatable, near-zero marginal cost** automation.

---

## Core Highlights

### Real Desktop Execution

Nuphus installs directly on your OS with native-grade screen perception and input control:

- **Drive any GUI** — window + OCR + mouse/keyboard; automate any desktop app or web page without needing their API
- **Built-in browser** — a programmable browser engine; web automation, data collection, and form interaction all run locally
- **Programming & project analysis** — project analysis, code generation, debugging, file operations with deep project context
- **Multi-agent orchestration** — run Cline, Claude Code and other external agents in parallel, consolidating decisions

### Dual-Device Sync: Desktop Works, Phone Controls

Nuphus separates "execution" from "control": **the desktop is the Agent's hands, your phone is the remote.**

- **Same session, synced across devices** — the phone connects to the same desktop Agent: messages share one entry point; history, memory and state are shared
- **Real-time event stream** — every step (thinking, tool calls, results) streams to the phone over WebSocket
- **Workflow remote control** — pause, resume, stop running workflows from your phone
- **Execution trace playback** — view the Agent's full trace on the phone; every step is transparent
- **Free remote access** — auto direct LAN connection (zero config); outside the LAN it routes through a relay server that never stores content, only authenticates and forwards

Auto channel switching: on the same WiFi the phone connects to the desktop directly (fast, free); leaving the LAN automatically switches to the relay (stable, reliable); returning switches back.

### Local-First, Private by Default

- **Data stays on your machine** — conversations, memory, plugins all stored locally
- **Local AI engines** — PP-OCRv4 (OCR) and Candle (semantic search) run locally; everyday recognition costs zero API calls
- **4-layer security** — permission gates → human-in-the-loop → injection detection → circuit breaker
- **Model freedom** — unified access to OpenAI / Anthropic / DeepSeek / Qwen / Zhipu and other mainstream providers, switch anytime

### Built-in Tools: PDF / Image / Video / Audio / Docs

No external tools to install — 23 built-in processing commands:

| Category | Capabilities |
|----------|--------------|
| PDF | Merge / Compress / Text extraction / Images to PDF / Extract pages / Rotate |
| Image | Compress / Convert / Resize / Stitch / Batch compress / Batch convert |
| Video | Compress / Extract frames / To GIF / Cut clip |
| Audio | Extract audio / Audio convert / **Voice clone** (cloud) |
| Docs | docx / pptx / xls / pdf → text |

### More Capabilities

| Capability | Description |
|------------|-------------|
| **Memory system** | Cross-session experience accumulation; SQLite + FTS5 + vector semantic search — gets smarter the more you use it |
| **Zero-compile extension** | Knowledge, skills, workflows, ui-maps are plain-text files; drop into `plugin/` and they load |
| **Three-layer vision** | Built-in OCR (zero API cost) → configurable vision model (complex scenes) → user-guided vision (most flexible fallback) |
| **Deterministic workflows** | Compile recurring tasks into workflows and run with zero tokens (see philosophy above) |
| **Sound feedback** | Task done / error / retry all have distinct sounds — know the state without staring at the screen |

---

## Installation

### One-command npm install (recommended)

A single command installs and auto-matches your platform's binary (Windows x64 / macOS arm64 / Linux x64). **No installer download, no Node.js / Rust required**:

```bash
# Global install (provides the nuphus command)
npm install -g @nuphus/nuphus-desktop

# Or try without installing (no global write)
npx @nuphus/nuphus-desktop
```

Then type `nuphus` in your terminal to launch.

> First install is large (desktop app bundles local OCR / speech models), please be patient.

### Download installers

For users unfamiliar with the command line — **no CLI, no Node.js / Rust required**:

1. Download the installer for your platform from [GitHub Releases](https://github.com/mrpulor-gh/nuphus/releases):
   - **Windows**: `.exe` (NSIS installer, per-user, **no admin rights needed**)
   - **macOS**: `.dmg`
   - **Linux**: `.deb` / `.AppImage`
2. Double-click to install (Windows creates a **Nuphus** desktop shortcut)
3. Double-click the shortcut to launch

### Build from source (developers)

**Prerequisites:**

| Tool | Version | Purpose |
|------|---------|---------|
| [Rust](https://rustup.rs/) | ≥ 1.95 | Core engine compilation |
| [Node.js](https://nodejs.org/) | ≥ 18 | Tauri frontend build |
| Tauri CLI | `cargo install tauri-cli --version "^2"` | Desktop app development |

```bash
git clone https://github.com/mrpulor-gh/nuphus.git
cd nuphus

# Install dependencies (root Tauri CLI + frontend deps)
npm install

# Run in dev mode
npx tauri dev
```

---

## Quick Start

### Configure a model

1. On first launch, follow the onboarding to pick a provider and enter your API key
2. Press Enter to finish

> You can also skip config with env vars: `QWEN_API_KEY="sk-xxx" npx tauri dev`

> After onboarding, modify config in **Settings → Models**. API keys are encrypted: on Windows they're encrypted with system-level DPAPI into local config.toml (`enc:v1:` format, bound to the current user); macOS/Linux currently store plaintext (relying on file permissions; OS credential integration is on the roadmap). Never share or commit this file.

### Connect your phone

1. On the desktop, enable the mobile service in the **Phone** settings page (default port 18772)
2. Open the pairing page in your phone browser, enter the pairing password
3. Add to Home Screen (PWA); use it like an app from then on

Auto LAN direct connection on the same WiFi; automatic relay channel outside the LAN — zero config end to end.

---

## Architecture Overview

Nuphus uses a six-layer architecture, security threaded through every layer:

```
┌─────────────────────────────────────────────┐
│ Tauri shell                                 │  ← frontend UI + OS capabilities (notify, tray, hotkeys)
├─────────────────────────────────────────────┤
│ Runtime                                     │  ← unified main loop, 3-mode routing (Leader / Workflow / Custom)
├─────────────────────────────────────────────┤
│ Agent                                       │  ← Leader decision / ExecAgent execution / WorkflowAgent design
├─────────────────────────────────────────────┤
│ Transport                                   │  ← multi-provider abstraction (unified access to major AI vendors)
├─────────────────────────────────────────────┤
│ Tools / Memory / Workflow                   │  ← execution infrastructure
├─────────────────────────────────────────────┤
│ Security / Permissions                      │  ← security chain across all layers (injection detection / permission tiers / review)
└─────────────────────────────────────────────┘
```

**Dual-device sync architecture**:

```
┌──────────┐   WebSocket real-time event stream   ┌──────────────┐
│  Phone PWA│ ←────────────────────────────────→  │ desktop mobile│
│(chat/remote)│   POST /message shared entry        │ server(18772)│
└──────────┘                                      └──────┬───────┘
      ↕ LAN direct (auto on same WiFi)                   │ shared session/memory/state
┌──────────┐                                      ┌──────┴───────┐
│ Relay    │ ←──── remote channel (free) ────→    │  Nuphus      │
│ server   │                                      │  desktop     │
│ (no store)│                                     │  Agent engine │
└──────────┘                                      └──────────────┘
```

Phone messages go through `submit_user_message(source="mobile")`, sharing the same `leader_agent` / busy lock / dedup logic as the desktop — the two devices are **two views of one Agent**, not two separate systems. Disconnects auto-reconnect with exponential backoff and re-pull history to fill gaps; no messages are lost.

**Data flow**: user input → Tauri event → Runtime routing → Leader decision → `task_dispatch` → ExecAgent execution → result return → frontend display (synced to desktop & phone)

---

## Configuration

Nuphus uses a TOML config file; `src/config/mod.rs::load_registry` searches by priority:

| # | Path | Use |
|---|------|-----|
| 1 | `<exe_dir>/config.toml` | Portable deployment |
| 2 | `./config.toml` | Development |
| 3 | `~/.config/nuphus/config.toml` | Linux/macOS user-level |
| 4 | `~/.nuphus/config.toml` | Legacy compatibility |
| 5 | `<AppData>/nuphus/providers.toml` | Windows desktop canonical config (anchored by desktop, auto-generated on first launch) |

---

## Design Principles

1. **Local-first** — data stays local by default; cloud only when you opt in
2. **Minimal mental model** — keep every feature simple, avoid over-abstraction
3. **Determinism first** — compile to a workflow instead of re-reasoning; reuse instead of rewriting
4. **Long-term first** — prefer solutions consistent with the existing architecture and maintainable
5. **Restraint over stacking** — every new feature must prove it's irreplaceable
6. **Closed-loop design** — every module forms a complete loop from input to output

---

## Contributing

Nuphus is a community-driven open-source project. Besides code, you can help build the ecosystem:

### Contribute plugins (coming soon)

| Plugin type | Description | Examples |
|-------------|-------------|----------|
| **ui-maps** | UI layout descriptions for any software (button positions, window identification) | Photoshop export panel, enterprise ERP layout |
| **workflows** | Reusable workflow templates | "Daily project backup", "Batch image compression" |
| **skills** | Domain methodologies and operation guides | "Frontend UI design guidelines", "Code patterns for a framework" |
| **knowledge** | Project domain knowledge docs | API references, config notes, architecture docs |

All plugins are plain-text files (.md / .json); drop them into the matching `plugin/` directory and Nuphus loads them.

### Get involved

- GitHub Issues: bug reports and feature requests
- GitHub Discussions: usage questions, experience sharing, plugin recommendations

---

## License

Copyright © 2026 Nuphus Team · Apache License 2.0