// The chats behind the main window: the sidebar's list, and the state of each chat you've
// opened, kept current from the core's events (including replies in chats you aren't looking
// at, and chats started elsewhere). Components subscribe to it like a Svelte store.

import type { CoreEvent, ImageData } from "../protocol.ts";
import type { Transport } from "../rpc/transport.ts";
import {
  type StoredMessage,
  type Transcript,
  type TranscriptInput,
  emptyTranscript,
  reduce,
} from "../transcript.ts";

/** A chat in the list (`lucy_protocol::rpc::SessionSummary`). */
export interface SessionSummary {
  id: string;
  title: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  message_count: number;
  /** No memory, and deleted when you leave it. */
  incognito?: boolean;
}

/** A chat's own settings (`lucy_protocol::rpc::SessionSettings`). */
export interface SessionSettings {
  profile: string;
  plain: boolean;
  mode: "assist" | "agent";
  /** Agent level, 0-3. */
  level: number;
}

export interface UndoResult {
  files: { path: string; created: boolean }[];
  applied: boolean;
}

export interface ChatState {
  transcript: Transcript;
  settings: SessionSettings | null;
  /** A reply is in progress. */
  busy: boolean;
  /** Something happened here since you last looked. */
  unread: boolean;
}

export interface ChatsState {
  sessions: SessionSummary[];
  selected: string | null;
  chats: Record<string, ChatState>;
  /** The open chat (or the next one you start) is incognito. */
  incognito: boolean;
}

type Decision = "once" | "always" | "deny";

export class ChatsStore {
  private readonly core: Transport;
  private state: ChatsState = { sessions: [], selected: null, chats: {}, incognito: false };
  private readonly subscribers = new Set<(state: ChatsState) => void>();
  private readonly pictures = new Map<string, Promise<string>>();

  constructor(core: Transport) {
    this.core = core;
    core.onEvent((event) => this.onEvent(event));
  }

  /** The Svelte store contract: called now and on every change. */
  subscribe(run: (state: ChatsState) => void): () => void {
    this.subscribers.add(run);
    run(this.state);
    return () => this.subscribers.delete(run);
  }

  /** Fetch the list of chats. */
  async load(): Promise<void> {
    const sessions = await this.core.call<SessionSummary[]>("sessions.list", {});
    this.update({ sessions: sortSessions(sessions) });
  }

  /** Show a chat, loading its history the first time. */
  async open(id: string): Promise<void> {
    if (id !== this.state.selected) {
      await this.leaveIncognito();
    }
    const known = this.state.chats[id];
    if (known === undefined) {
      const [session, settings] = await Promise.all([
        this.core.call<{ summary: SessionSummary; messages: StoredMessage[] }>("sessions.load", {
          session_id: id,
        }),
        this.core.call<SessionSettings>("session.settings", { session_id: id }),
      ]);
      const transcript = reduce(emptyTranscript(), {
        type: "history",
        messages: session.messages,
        live: null,
      });
      this.upsertSession(session.summary);
      this.setChat(id, { transcript, settings, busy: false, unread: false });
    } else {
      this.setChat(id, { ...known, unread: false });
    }
    this.update({ selected: id });
  }

  /** Close the open chat (the next message starts a new one, incognito if asked). */
  async newChat(incognito = false): Promise<void> {
    await this.leaveIncognito();
    this.update({ selected: null, incognito });
  }

  /** Leaving an incognito chat deletes it. */
  private async leaveIncognito(): Promise<void> {
    const id = this.state.selected;
    if (!this.state.incognito || id === null) {
      return;
    }
    await this.core.call("sessions.delete", { session_id: id }).catch(() => {});
    const { [id]: _gone, ...chats } = this.state.chats;
    this.update({
      sessions: this.state.sessions.filter((s) => s.id !== id),
      chats,
      selected: null,
      incognito: false,
    });
  }

  /**
   * Send a message (with any pictures) in the open chat, or start a new chat with it.
   * `/image <description>` makes a picture straight away instead; `/clear` and `/compress`
   * start the model afresh or summarise the chat so far (the chat itself keeps everything).
   */
  async send(text: string, images: ImageData[] = []): Promise<void> {
    const selected = this.state.selected;
    const command = images.length === 0 ? /^\/(clear|compress)$/.exec(text.trim())?.[1] : undefined;
    if (command !== undefined) {
      if (selected !== null) {
        await this.chatCommand(selected, `chat.${command}`);
      }
      return;
    }
    const chat = selected === null ? {} : { session_id: selected };
    const prompt = images.length === 0 ? /^\/image\s+(.+)$/s.exec(text)?.[1]?.trim() : undefined;
    try {
      const [method, params] =
        prompt !== undefined
          ? ["images.generate", { ...chat, prompt }]
          : [
              "chat.send",
              {
                ...chat,
                text,
                ...(images.length > 0 ? { images } : {}),
                ...(selected === null && this.state.incognito ? { incognito: true } : {}),
              },
            ];
      const sent = await this.core.call<{ session_id: string; turn_id: number }>(method, params);
      if (selected === null) {
        await this.open(sent.session_id);
      }
    } catch (err) {
      if (selected !== null) {
        this.apply(selected, { type: "problem", message: messageOf(err) });
      }
    }
  }

  /** `chat.clear` or `chat.compress`, then show the chat with its new marker. */
  private async chatCommand(id: string, method: string): Promise<void> {
    try {
      await this.core.call(method, { session_id: id });
      await this.reload(id);
    } catch (err) {
      this.apply(id, { type: "problem", message: messageOf(err) });
    }
  }

  /** A picture kept with a chat, as a `data:` URL an `<img>` can show (fetched once). */
  picture(id: string): Promise<string> {
    let url = this.pictures.get(id);
    if (url === undefined) {
      url = this.core
        .call<ImageData>("images.get", { id })
        .then((image) => `data:${image.media_type};base64,${image.data}`);
      // A failed fetch can be tried again later.
      url.catch(() => this.pictures.delete(id));
      this.pictures.set(id, url);
    }
    return url;
  }

  /** Stop the open chat's reply. */
  async cancel(): Promise<void> {
    const id = this.state.selected;
    if (id !== null) {
      await this.core.call("chat.cancel", { session_id: id });
    }
  }

  /** Answer the open chat's approval request. */
  async approve(callId: string, decision: Decision): Promise<void> {
    const id = this.state.selected;
    if (id === null) {
      return;
    }
    this.apply(id, { type: "answered", callId });
    try {
      await this.core.call("tool.approve", { session_id: id, call_id: callId, decision });
    } catch (err) {
      this.apply(id, { type: "problem", message: messageOf(err) });
    }
  }

  setMode(mode: SessionSettings["mode"]): Promise<void> {
    return this.changeSettings("session.set_mode", { mode });
  }
  setLevel(level: number): Promise<void> {
    return this.changeSettings("session.set_level", { level });
  }
  setProfile(profile: string): Promise<void> {
    return this.changeSettings("session.set_profile", { profile });
  }

  /** Make the open chat's model the default for new chats (and the sprite), like `/model --default`. */
  makeDefault(): Promise<void> {
    const id = this.state.selected;
    const profile = id === null ? undefined : this.state.chats[id]?.settings?.profile;
    if (profile === undefined) {
      return Promise.resolve();
    }
    return this.changeSettings("session.set_profile", { profile, make_default: true });
  }
  setPlain(plain: boolean): Promise<void> {
    return this.changeSettings("session.set_plain", { plain });
  }

  /** What the open chat's last change touched; with `apply`, put the files back. */
  async undo(apply: boolean): Promise<UndoResult | null> {
    const id = this.state.selected;
    if (id === null) {
      return null;
    }
    return this.core.call<UndoResult>("session.undo", { session_id: id, apply });
  }

  async rename(id: string, title: string): Promise<void> {
    await this.core.call("sessions.rename", { session_id: id, title });
    this.update({
      sessions: this.state.sessions.map((s) => (s.id === id ? { ...s, title } : s)),
    });
  }

  async remove(id: string): Promise<void> {
    await this.core.call("sessions.delete", { session_id: id });
    const { [id]: _gone, ...chats } = this.state.chats;
    this.update({
      sessions: this.state.sessions.filter((s) => s.id !== id),
      chats,
      selected: this.state.selected === id ? null : this.state.selected,
    });
  }

  // --- Internals -------------------------------------------------------------------------------

  private async changeSettings(method: string, change: Record<string, unknown>): Promise<void> {
    const id = this.state.selected;
    const chat = id === null ? undefined : this.state.chats[id];
    if (id === null || chat === undefined) {
      return;
    }
    try {
      const settings = await this.core.call<SessionSettings>(method, { session_id: id, ...change });
      this.setChat(id, { ...chat, settings });
    } catch (err) {
      this.apply(id, { type: "problem", message: messageOf(err) });
    }
  }

  private onEvent(event: CoreEvent): void {
    const id = typeof event.session_id === "string" ? event.session_id : null;
    if (id === null) {
      return;
    }
    if (!this.state.sessions.some((s) => s.id === id)) {
      // A chat started elsewhere: fetch the list to learn its title and place.
      this.load().catch(() => {});
    } else if (event.type === "user_message" || event.type === "turn_finished") {
      this.touch(id);
    }
    const known = this.state.chats[id];
    if (known === undefined) {
      return;
    }
    if (event.type === "turn_finished" && known.transcript.turnId !== event.turn_id) {
      // A reply that began before this chat was opened: its text never reached us, but it's
      // saved now, so show the chat as saved.
      this.reload(id).catch(() => {});
      return;
    }
    this.apply(id, { type: "event", event });
    const chat = this.state.chats[id];
    if (chat !== undefined && id !== this.state.selected && event.type !== "user_message") {
      this.setChat(id, { ...chat, unread: true });
    }
  }

  private async reload(id: string): Promise<void> {
    const session = await this.core.call<{ summary: SessionSummary; messages: StoredMessage[] }>(
      "sessions.load",
      { session_id: id },
    );
    const chat = this.state.chats[id];
    if (chat === undefined) {
      return;
    }
    const transcript = reduce(emptyTranscript(), {
      type: "history",
      messages: session.messages,
      live: null,
    });
    this.setChat(id, { ...chat, transcript, busy: false });
  }

  private apply(id: string, input: TranscriptInput): void {
    const chat = this.state.chats[id];
    if (chat === undefined) {
      return;
    }
    const transcript = reduce(chat.transcript, input);
    this.setChat(id, { ...chat, transcript, busy: transcript.turnId !== null });
  }

  /** Move a chat to the top of the list (it just changed). */
  private touch(id: string): void {
    const now = Date.now();
    this.update({
      sessions: sortSessions(
        this.state.sessions.map((s) => (s.id === id ? { ...s, updated_at_ms: now } : s)),
      ),
    });
  }

  private upsertSession(summary: SessionSummary): void {
    const others = this.state.sessions.filter((s) => s.id !== summary.id);
    this.update({ sessions: sortSessions([...others, summary]) });
  }

  private setChat(id: string, chat: ChatState): void {
    this.update({ chats: { ...this.state.chats, [id]: chat } });
  }

  private update(change: Partial<ChatsState>): void {
    this.state = { ...this.state, ...change };
    for (const run of this.subscribers) {
      run(this.state);
    }
  }
}

function sortSessions(sessions: SessionSummary[]): SessionSummary[] {
  return [...sessions].sort((a, b) => b.updated_at_ms - a.updated_at_ms);
}

function messageOf(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
