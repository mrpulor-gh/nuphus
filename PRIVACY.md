# Privacy Policy

Nuphus is designed with **local-first privacy**. This document describes what data is processed, where it goes, and what never leaves your machine.

## Data That Stays Local (Never Leaves Your Device)

| Data | Storage | Details |
|------|---------|---------|
| **API Keys** | config.toml | Encrypted at rest with Windows DPAPI (`enc:v1:` format, bound to your user account); stored in plain text on macOS/Linux pending OS keychain support. Treat this file like a password — do not share or commit it. |
| **Chat Sessions** | SQLite (~/.nuphus/) | All conversation history, including messages, tool calls, and execution results. |
| **Memory** | SQLite (FTS5 + vector index) | Cross-session experience records, annotations, and learned patterns. |
| **Workflow Definitions** | plugin/workflows/ | Workflow JSON definitions and execution records. |
| **UI Maps** | plugin/ui-maps/ | Screen layout recognition data for desktop automation. |
| **OCR Engine** | src-tauri/desktop/models/ | ONNX OCR models (PP-OCRv4). Auto-fetched at build time via src-tauri/build.rs. Local-only inference, never sent to external services. |
| **Screenshots** | Temp directory | Desktop screenshots for OCR / automation. Automatically deleted after processing. |
| **Logs** | 
uphus-*.log | Debug logs, configurable via RUST_LOG. Rotated automatically. |

## Data That Is Sent to AI Providers

When you send a message or execute a task, the **conversation context** (your message + relevant history + tool results) is sent to the AI provider you configured (e.g., OpenAI, Anthropic, DeepSeek, etc.).

This is the fundamental function of Nuphus as an AI agent — it needs to communicate with LLM APIs to operate.

**What is NOT sent to AI providers:**
- Your API keys
- Your config.toml
- Files you haven't explicitly asked Nuphus to read
- Desktop screenshots (unless the current task requires OCR)
- Memory records (unless referenced by the current context)

## Third-Party Services

Nuphus communicates only with:
1. **AI providers you configure** — API calls to the base URLs you specify
2. **Web search** — When you explicitly use the web_search tool (configurable)
3. **Web extraction** — When you explicitly use the web_extract tool

No other third-party services are contacted. **No telemetry, no analytics, no crash reporting.**

## AI Providers

Nuphus supports connecting to multiple AI providers via config.toml. All major providers are supported:

- OpenAI / Anthropic / DeepSeek / Alibaba Cloud (Qwen) / Zhipu (GLM) / MiniMax / Kimi
- Any OpenAI-compatible API endpoint (custom base_url)

Each provider has its own data handling policy. Please refer to the respective provider's privacy policy for how they process data sent to their APIs.

## Security

- API keys are encrypted at rest via Windows DPAPI (`enc:v1:` format, user-bound); macOS/Linux currently store keys in plain text (OS file permissions apply), with OS keychain integration on the roadmap — this is a deliberate trade-off, not an oversight: there is no cross-platform OS enclave API, and per-platform keychain integration adds authorization prompts plus headless/CI failure paths. An honestly documented plain-text fallback beats a veneer of protection
- Tool execution is subject to permission policies and injection detection
- File operations are sandboxed by configurable path scopes
- Desktop automation requires explicit user approval for high-risk operations

## Updates

Nuphus does not check for updates automatically. You control when and how to update.

## Contact

For privacy concerns or questions, please [open an issue](https://github.com/mrpulor-gh/nuphus/issues).

---

*Last updated: 2026-08-23*