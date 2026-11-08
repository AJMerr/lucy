// The settings page: config.toml split into sections you edit and save one at a time (each
// Save sends only what changed, as a JSON merge patch, and the core checks the whole result),
// plus API keys and the persona, which have their own core methods.

import type { Transport } from "../rpc/transport.ts";

/** config.toml as the core sends it (`config.get`), loosely typed: forms know their fields. */
// biome-ignore lint: the config's shape is the core's to define.
export type ConfigJson = Record<string, any>;

/** Which top-level config tables each section edits. */
export const SECTIONS = {
  models: ["defaults", "profiles"],
  agent: ["agent", "tools", "user"],
  mcp: ["mcp"],
  sprite: ["sprite", "app"],
  remote: ["remote"],
} as const;

export type SectionId = keyof typeof SECTIONS;

export interface Section {
  /** What the form shows and edits. */
  draft: ConfigJson;
  /** It differs from what's saved. */
  dirty: boolean;
  saving: boolean;
  /** Why the last save was refused. */
  error: string | null;
}

export interface KeyStatus {
  name: string;
  used_by: string[];
  /** `keyring`, an environment variable's name, or null when missing. */
  source: string | null;
  error?: string;
}

export interface PersonaView {
  name: string;
  text: string;
  overridden: boolean;
  path: string | null;
}

export interface SettingsState {
  loaded: boolean;
  /** Where config.toml lives on the core's machine. */
  path: string;
  /** home-manager owns config.toml, so nothing can be saved from here. */
  managed: boolean;
  /** The saved config; null before first-run setup. */
  config: ConfigJson | null;
  sections: Record<SectionId, Section>;
  keys: KeyStatus[];
  persona: PersonaView | null;
  /** A saved change only applies once the core restarts. */
  restartNeeded: boolean;
}

const MANAGED =
  "config.toml is managed by home-manager (programs.lucy.settings); change it in your Nix config";

export class SettingsStore {
  private readonly core: Transport;
  private state: SettingsState = {
    loaded: false,
    path: "",
    managed: false,
    config: null,
    sections: emptySections(),
    keys: [],
    persona: null,
    restartNeeded: false,
  };
  private readonly subscribers = new Set<(state: SettingsState) => void>();

  constructor(core: Transport) {
    this.core = core;
  }

  subscribe(run: (state: SettingsState) => void): () => void {
    this.subscribers.add(run);
    run(this.state);
    return () => this.subscribers.delete(run);
  }

  /** Fetch everything the page shows. */
  async load(): Promise<void> {
    const view = await this.core.call<{ path: string; managed: boolean; config: ConfigJson | null }>(
      "config.get",
    );
    const sections = emptySections();
    for (const id of sectionIds()) {
      sections[id] = { draft: slice(view.config, id), dirty: false, saving: false, error: null };
    }
    this.update({
      loaded: true,
      path: view.path,
      managed: view.managed,
      config: view.config,
      sections,
    });
    if (view.config !== null) {
      await Promise.all([this.refreshKeys(), this.refreshPersona()]);
    }
  }

  /** Change a section's form; `change` edits a copy of its draft in place. */
  edit(id: SectionId, change: (draft: ConfigJson) => void): void {
    const draft = structuredClone(this.state.sections[id].draft);
    change(draft);
    const dirty = mergePatch(slice(this.state.config, id), draft) !== undefined;
    this.setSection(id, { ...this.state.sections[id], draft, dirty });
  }

  revert(id: SectionId): void {
    this.setSection(id, { draft: slice(this.state.config, id), dirty: false, saving: false, error: null });
  }

  /** Send a section's changes; the core refuses anything that would make an invalid config. */
  async save(id: SectionId): Promise<void> {
    const section = this.state.sections[id];
    if (this.state.managed) {
      this.setSection(id, { ...section, error: MANAGED });
      return;
    }
    const patch = mergePatch(slice(this.state.config, id), section.draft);
    if (patch === undefined) {
      return;
    }
    this.setSection(id, { ...section, saving: true, error: null });
    try {
      const done = await this.core.call<{ restart_needed: boolean }>("config.update", { patch });
      const view = await this.core.call<{ config: ConfigJson | null }>("config.get");
      this.update({ config: view.config, restartNeeded: this.state.restartNeeded || done.restart_needed });
      this.setSection(id, { draft: slice(view.config, id), dirty: false, saving: false, error: null });
      // Profiles and MCP servers decide which keys exist.
      await this.refreshKeys();
    } catch (err) {
      this.setSection(id, { ...this.state.sections[id], saving: false, error: messageOf(err) });
    }
  }

  /** Store an API key in the keyring of the core's machine. The value isn't kept here. */
  async setKey(name: string, value: string): Promise<void> {
    await this.core.call("keys.set", { name, value });
    await this.refreshKeys();
  }

  async removeKey(name: string): Promise<void> {
    await this.core.call("keys.remove", { name });
    await this.refreshKeys();
  }

  async savePersona(text: string): Promise<void> {
    await this.core.call("persona.set", { text });
    await this.refreshPersona();
  }

  async resetPersona(): Promise<void> {
    await this.core.call("persona.reset");
    await this.refreshPersona();
  }

  /** Sign in to a remote MCP server (the browser opens on the core's machine). */
  async signIn(server: string): Promise<void> {
    await this.core.call("mcp.login", { name: server });
  }

  private async refreshKeys(): Promise<void> {
    try {
      this.update({ keys: await this.core.call<KeyStatus[]>("keys.status") });
    } catch {
      this.update({ keys: [] });
    }
  }

  private async refreshPersona(): Promise<void> {
    try {
      this.update({ persona: await this.core.call<PersonaView>("persona.get") });
    } catch {
      this.update({ persona: null });
    }
  }

  private setSection(id: SectionId, section: Section): void {
    this.update({ sections: { ...this.state.sections, [id]: section } });
  }

  private update(change: Partial<SettingsState>): void {
    this.state = { ...this.state, ...change };
    for (const run of this.subscribers) {
      run(this.state);
    }
  }
}

/**
 * The JSON merge patch (RFC 7396) that turns `before` into `after`: changed and new values,
 * and null for what's gone; `undefined` when nothing changed.
 */
export function mergePatch(before: unknown, after: unknown): unknown {
  if (isObject(before) && isObject(after)) {
    const patch: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(after)) {
      if (value === undefined) {
        continue;
      }
      if (!(key in before)) {
        patch[key] = value;
        continue;
      }
      const inner = mergePatch(before[key], value);
      if (inner !== undefined) {
        patch[key] = inner;
      }
    }
    for (const key of Object.keys(before)) {
      if (after[key] === undefined) {
        patch[key] = null;
      }
    }
    return Object.keys(patch).length === 0 ? undefined : patch;
  }
  if (after === undefined) {
    return null;
  }
  return JSON.stringify(before) === JSON.stringify(after) ? undefined : after;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sectionIds(): SectionId[] {
  return Object.keys(SECTIONS) as SectionId[];
}

function emptySections(): Record<SectionId, Section> {
  const sections = {} as Record<SectionId, Section>;
  for (const id of sectionIds()) {
    sections[id] = { draft: {}, dirty: false, saving: false, error: null };
  }
  return sections;
}

/**
 * The part of the config a section edits, with the tables the core leaves out when empty
 * filled in, so a form always has somewhere to put things.
 */
function slice(config: ConfigJson | null, id: SectionId): ConfigJson {
  const part: ConfigJson = {};
  for (const key of SECTIONS[id]) {
    part[key] = structuredClone(config?.[key] ?? {});
  }
  if (id === "mcp") {
    part.mcp.servers ??= {};
  }
  return part;
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
