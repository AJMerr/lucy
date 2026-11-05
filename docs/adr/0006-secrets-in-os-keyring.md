# 6. API keys in the OS keyring

Date: 2026-11-05 · Status: Accepted

## Decision

- Keys are stored in the OS keyring (Secret Service / Keychain / Credential Manager) via
  `lucy auth set <key_name>`.
- An environment variable `LUCY_KEY_<KEY_NAME>` overrides the keyring (useful for CI and
  headless boxes). Vendor-standard vars (e.g. `ANTHROPIC_API_KEY`) are accepted as a fallback.
- `config.toml` only ever references a `key_name`, never a key.

## Consequences

- Headless Linux without a Secret Service daemon must use the env var path.
