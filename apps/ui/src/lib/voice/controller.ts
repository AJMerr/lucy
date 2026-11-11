// The voice pipeline on a page (the sprite, or the main window): what you say becomes a message,
// and the reply to it (or to what you typed, if replies are read aloud) is spoken a sentence at
// a time as it streams in. Talking to her, or Stop, cuts her off. The engine (in the desktop
// app) does the hearing and speaking.

import type { CoreEvent } from "../protocol.ts";
import { SpeechStream } from "./speech.ts";

/** Lucy's app's voice: the microphone, Whisper, and her voice. */
export interface VoiceEngine {
  /** Start listening: `hold` until `stopListening` (push-to-talk), else until you go quiet. */
  listen(hold: boolean): Promise<void>;
  /** Stop listening; what you said arrives as `heard`. */
  stopListening(): Promise<void>;
  /** Say this after anything already queued. */
  say(text: string): Promise<void>;
  /** Stop talking now, dropping what's queued. */
  hush(): Promise<void>;
}

export interface VoiceOptions {
  /** Send a message in this page's chat. */
  send: (text: string) => Promise<void>;
  /** Read aloud replies to typed messages too. */
  readTyped: () => boolean;
}

export class VoiceController {
  private readonly engine: VoiceEngine;
  private readonly options: VoiceOptions;
  /** The next reply to start is to be spoken. */
  private speakNext = false;
  /** The reply being spoken: its turn, text so far and sentences said. */
  private speaking: { turnId: number; text: string; stream: SpeechStream } | null = null;

  constructor(engine: VoiceEngine, options: VoiceOptions) {
    this.engine = engine;
    this.options = options;
  }

  /** You spoke: send it, and speak the reply. */
  async heard(text: string): Promise<void> {
    const message = text.trim();
    if (message === "") {
      return;
    }
    this.speakNext = true;
    await this.options.send(message);
  }

  /** You typed and sent a message on this page. */
  typed(): void {
    this.speakNext = this.options.readTyped();
  }

  /** Push-to-talk (or the wake word): cut her off and listen. */
  async talk(hold: boolean): Promise<void> {
    await this.hush();
    await this.engine.listen(hold);
  }

  async stopTalking(): Promise<void> {
    await this.engine.stopListening();
  }

  /** Stop speaking the current reply. */
  async hush(): Promise<void> {
    this.speaking = null;
    await this.engine.hush();
  }

  /** The page's chat events. */
  onEvent(event: CoreEvent): void {
    if (event.type === "turn_started") {
      if (this.speakNext && typeof event.turn_id === "number") {
        this.speaking = { turnId: event.turn_id, text: "", stream: new SpeechStream() };
      }
      this.speakNext = false;
      return;
    }
    const speaking = this.speaking;
    if (speaking === null || event.turn_id !== speaking.turnId) {
      return;
    }
    switch (event.type) {
      case "delta":
        speaking.text += event.text ?? "";
        this.say(speaking.stream.feed(speaking.text));
        break;
      // Text before a tool call is a reply of its own: say the rest, then start afresh.
      case "tool_started":
        this.say(speaking.stream.finish(speaking.text));
        speaking.text = "";
        speaking.stream = new SpeechStream();
        break;
      case "turn_finished":
        this.speaking = null;
        if (event.outcome === "cancelled") {
          void this.engine.hush();
        } else {
          this.say(speaking.stream.finish(speaking.text));
        }
        break;
    }
  }

  private say(sentences: string[]): void {
    for (const sentence of sentences) {
      void this.engine.say(sentence);
    }
  }
}
