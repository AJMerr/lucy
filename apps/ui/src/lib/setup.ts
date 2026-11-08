// First-run setup, like `lucy config init`: the provider presets come from the core
// (`setup.presets`, the same list the terminal wizard uses), and the answers become the first
// config.toml plus, for hosted providers, an API key in the keyring.

import type { Transport } from "./rpc/transport.ts";
import type { ConfigJson } from "./stores/settings.ts";

/** A model provider to offer (`lucy_protocol::rpc::ProviderPreset`). */
export interface ProviderPreset {
  label: string;
  profile_name: string;
  provider: "openai_compat" | "anthropic" | string;
  /** Null for Anthropic, and for a custom endpoint (you type one). */
  base_url: string | null;
  /** The suggested model; null for a custom endpoint. */
  model: string | null;
  /** Where its key goes in the keyring; null when it needs none. */
  key_name: string | null;
}

export interface SetupAnswers {
  preset: ProviderPreset;
  profileName: string;
  model: string;
  /** A custom endpoint's URL. */
  baseUrl?: string;
  /** A custom endpoint's key name. */
  keyName?: string;
  /** The API key itself, if the provider takes one. */
  key?: string;
  /** What Lucy calls you. */
  userName?: string;
}

/** The config change that sets up a first profile from the answers. */
export function setupPatch(answers: SetupAnswers): ConfigJson {
  const { preset } = answers;
  const custom = preset.model === null;
  const baseUrl = custom ? answers.baseUrl?.trim() : preset.base_url;
  const keyName = custom ? answers.keyName?.trim() : preset.key_name;
  const profile: ConfigJson = { provider: preset.provider };
  if (baseUrl) {
    profile.base_url = baseUrl;
  }
  profile.model = answers.model.trim();
  if (keyName) {
    profile.key_name = keyName;
  }
  const name = answers.profileName.trim();
  const patch: ConfigJson = { defaults: { profile: name }, profiles: { [name]: profile } };
  const userName = answers.userName?.trim();
  if (userName) {
    patch.user = { name: userName };
  }
  return patch;
}

/** Write the first config, then store the key (if one was typed) under the profile's name. */
export async function completeSetup(core: Transport, answers: SetupAnswers): Promise<void> {
  const patch = setupPatch(answers);
  await core.call("config.update", { patch });
  const key = answers.key?.trim();
  const keyName = patch.profiles[answers.profileName.trim()].key_name as string | undefined;
  if (key && keyName) {
    await core.call("keys.set", { name: keyName, value: key });
  }
}
