# 2. Own `Provider` trait instead of a third-party LLM framework

Date: 2026-11-05 · Status: Accepted

## Context

Lucy must not depend on any one model vendor. Required on day one: OpenAI-compatible APIs
(OpenAI, Ollama, Qwen/DashScope, OpenRouter, LM Studio, vLLM...) and Anthropic. Rust options
were `genai`, `rig`, or our own abstraction.

## Decision

Define a small `Provider` trait in `lucy-providers` (streamed chat, then tool calls) with
adapters:

- `mock` — scripted replies for offline tests
- `openai_compat` — one adapter, many backends via `base_url`
- `anthropic` — native Messages API

Users configure named **profiles** (provider + base URL + model + key name) in `config.toml`
and switch with `/model <profile>`. There is no built-in default; a first-run wizard creates
the first profile.

## Consequences

- Full control over streaming and tool-call semantics; no upstream release coupling.
- We maintain the adapters ourselves. Adding Gemini or others later = one new adapter.
- Adapters are tested against recorded real responses; live tests are opt-in.
