// Voice in Lucy's desktop app: the commands and events of src-tauri/src/voice.rs, as the pages
// use them. Absent outside the desktop app (voice needs its microphone and speakers).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { VoiceEngine } from "./controller.ts";

/** `voice_status`. */
export interface VoiceStatus {
  enabled: boolean;
  /** Downloaded and loaded. */
  ready: boolean;
  /** Why it isn't ready, if it failed. */
  error: string | null;
  read_typed: boolean;
  wake_word: boolean;
}

export interface Progress {
  label: string;
  done: number;
  total: number;
}

/** What Settings → Voice needs from the app. */
export interface VoiceApp {
  /** Read the settings again, download what's needed and start voice. */
  prepare(): Promise<VoiceStatus>;
  say(text: string): Promise<void>;
  onProgress(run: (progress: Progress) => void): () => void;
}

/** The desktop app's voice. */
export class DesktopVoice implements VoiceApp, VoiceEngine {
  /** Which page is listening: "sprite" or "window". */
  private readonly origin: string;

  constructor(origin: string) {
    this.origin = origin;
  }

  status(): Promise<VoiceStatus> {
    return invoke<VoiceStatus>("voice_status");
  }
  prepare(): Promise<VoiceStatus> {
    return invoke<VoiceStatus>("voice_prepare");
  }
  listen(hold: boolean): Promise<void> {
    return invoke("voice_listen", { hold, origin: this.origin });
  }
  stopListening(): Promise<void> {
    return invoke("voice_stop_listening");
  }
  say(text: string): Promise<void> {
    return invoke("voice_say", { text });
  }
  hush(): Promise<void> {
    return invoke("voice_hush");
  }

  onProgress(run: (progress: Progress) => void): () => void {
    return subscribe<Progress>("voice-progress", run);
  }
  /** What you said, if this page was listening. */
  onHeard(run: (text: string) => void): () => void {
    return subscribe<{ origin: string; text: string }>("voice-heard", (heard) => {
      if (heard.origin === this.origin) {
        run(heard.text);
      }
    });
  }
  onState(run: (state: "listening" | "thinking" | "idle") => void): () => void {
    return subscribe("voice-state", run);
  }
  onWake(run: () => void): () => void {
    return subscribe("voice-wake", run);
  }
  /** Voice was started, restarted (after Settings) or turned off. */
  onStatus(run: (status: VoiceStatus) => void): () => void {
    return subscribe<VoiceStatus>("voice-status", run);
  }
  onError(run: (message: string) => void): () => void {
    return subscribe<{ message: string }>("voice-error", (e) => run(e.message));
  }
}

function subscribe<T>(event: string, run: (payload: T) => void): () => void {
  let stop = () => {};
  let stopped = false;
  listen<T>(event, (e) => run(e.payload)).then((unlisten) => {
    if (stopped) {
      unlisten();
    } else {
      stop = unlisten;
    }
  });
  return () => {
    stopped = true;
    stop();
  };
}
