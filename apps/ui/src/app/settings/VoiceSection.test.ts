// Voice: talk to Lucy and hear her. Choosing how she hears and sounds, a hosted voice's key,
// reading typed replies aloud and the wake word; saving gets the app's voice ready.

import assert from "node:assert/strict";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, test } from "vitest";

import { SettingsStore } from "../../lib/stores/settings.ts";
import { FakeCore } from "../../lib/testing.ts";
import type { VoiceApp, VoiceStatus } from "../../lib/voice/app.ts";
import VoiceSection from "./VoiceSection.svelte";

afterEach(cleanup);

class FakeVoice implements VoiceApp {
  prepared = 0;
  said: string[] = [];
  progress: ((p: { label: string; done: number; total: number }) => void) | null = null;
  status: VoiceStatus = { enabled: false, ready: false, error: null, read_typed: false, wake_word: false };
  async prepare(): Promise<VoiceStatus> {
    this.prepared++;
    this.progress?.({ label: "Whisper small", done: 40, total: 100 });
    this.status = { ...this.status, enabled: true, ready: true };
    return this.status;
  }
  async say(text: string): Promise<void> {
    this.said.push(text);
  }
  onProgress(run: (p: { label: string; done: number; total: number }) => void): () => void {
    this.progress = run;
    return () => {};
  }
}

async function setUp(voiceConfig: Record<string, unknown> = {}, voice: FakeVoice | null = new FakeVoice()) {
  const core = new FakeCore();
  core.answers["config.get"] = {
    path: "/c.toml",
    managed: false,
    config: { defaults: { profile: "local" }, profiles: {}, voice: voiceConfig },
  };
  core.answers["keys.status"] = [];
  core.answers["persona.get"] = { name: "lucy", text: "", overridden: false, path: null };
  core.answers["memory.get"] = { path: "/m.md", text: "", facts: [] };
  core.answers["config.update"] = { restart_needed: false };
  core.answers["keys.set"] = null;
  const store = new SettingsStore(core);
  await store.load();
  render(VoiceSection, { store, voice: voice ?? undefined });
  return { core, voice };
}

test("turning voice on with a Piper voice saves it, then gets voice ready", async () => {
  const { core, voice } = await setUp();

  await fireEvent.click(screen.getByLabelText("Talk to Lucy"));
  await fireEvent.change(screen.getByLabelText("Her voice"), { target: { value: "piper" } });
  await fireEvent.change(screen.getByLabelText("Which voice"), {
    target: { value: "en_GB-jenny_dioco-medium" },
  });
  await fireEvent.change(screen.getByLabelText("Hearing"), { target: { value: "base" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  await waitFor(() => assert.equal(voice?.prepared, 1));
  assert.deepEqual(core.sent("config.update"), [
    {
      patch: {
        voice: { enabled: true, voice: "piper", speaker: "en_GB-jenny_dioco-medium", hearing: "base" },
      },
    },
  ]);
  await waitFor(() => screen.getByText(/Ready/));
});

test("reading aloud and the wake word wait for Talk to Lucy", async () => {
  await setUp({ read_typed: true });

  const readTyped = screen.getByLabelText("Read typed replies aloud") as HTMLInputElement;
  const wakeWord = screen.getByLabelText("Listen for Hey Lucy") as HTMLInputElement;
  assert.ok(readTyped.disabled);
  assert.ok(wakeWord.disabled);
  screen.getByText(/Turn on Talk to Lucy to use these/);

  await fireEvent.click(screen.getByLabelText("Talk to Lucy"));
  assert.ok(!readTyped.disabled);
  assert.ok(!wakeWord.disabled);
  assert.equal(screen.queryByText(/Turn on Talk to Lucy to use these/), null);
});

test("downloads show how far they've got", async () => {
  await setUp({ enabled: true });

  await fireEvent.click(screen.getByLabelText("Read typed replies aloud"));
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));

  await waitFor(() => screen.getByText(/Whisper small.*40%/));
});

test("a hosted voice takes its key, sent to the keyring", async () => {
  const { core } = await setUp({ enabled: true, voice: "openai" });

  assert.equal((screen.getByLabelText("Which voice") as HTMLSelectElement).value, "nova");
  await fireEvent.input(screen.getByLabelText("OpenAI API key"), { target: { value: "sk-test" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save key" }));

  await waitFor(() => assert.deepEqual(core.sent("keys.set"), [{ name: "openai", value: "sk-test" }]));
});

test("preview says hello in the chosen voice", async () => {
  const { voice } = await setUp({ enabled: true });

  await fireEvent.click(screen.getByRole("button", { name: "Preview" }));

  await waitFor(() => assert.equal(voice?.said.length, 1));
});

test("outside the desktop app it says voice lives there", async () => {
  await setUp({}, null);

  assert.ok(screen.getByText(/in Lucy's desktop app/));
  assert.equal(screen.queryByRole("button", { name: "Preview" }), null);
});
