// Which frame of which state the sprite shows at a given moment. Pure timing: the renderer
// asks for a frame on every animation tick and draws it.

/** One state of a pack, as far as timing goes. */
export interface StateSpec {
  /** How many frames it has. */
  frames: number;
  fps: number;
  loop: boolean;
  /** For a one-shot state (`loop: false`): the state to play once it's done. */
  then?: string;
}

export type States = Record<string, StateSpec>;

export interface Shown {
  state: string;
  index: number;
}

/** States the pack doesn't have fall back to this one, which every pack has. */
const FALLBACK = "idle";

/** One-shot states chained through `then` are followed at most this far per call. */
const MAX_HANDOVERS = 16;

export class Animator {
  #states: States;
  #state = FALLBACK;
  #since: number;

  constructor(states: States, now: number) {
    this.#states = states;
    this.#since = now;
  }

  /** Switch to `state` (from its first frame) unless it's already playing. */
  play(state: string, now: number): void {
    this.#settle(now);
    const next = this.#resolve(state);
    if (next !== this.#state) {
      this.#state = next;
      this.#since = now;
    }
  }

  /** The frame to draw at `now`. */
  frame(now: number): Shown {
    this.#settle(now);
    const spec = this.#spec(this.#state);
    const step = Math.floor((Math.max(0, now - this.#since) * spec.fps) / 1000);
    const index = spec.loop ? step % spec.frames : Math.min(step, spec.frames - 1);
    return { state: this.#state, index };
  }

  /** Follow one-shot states that have finished by `now` to their `then`. */
  #settle(now: number): void {
    for (let handovers = 0; handovers < MAX_HANDOVERS; handovers++) {
      const spec = this.#spec(this.#state);
      if (spec.loop || spec.then === undefined) {
        return;
      }
      const end = this.#since + (spec.frames * 1000) / spec.fps;
      if (now < end) {
        return;
      }
      this.#state = this.#resolve(spec.then);
      this.#since = end;
    }
  }

  #resolve(state: string): string {
    return state in this.#states ? state : FALLBACK;
  }

  #spec(state: string): StateSpec {
    const spec = this.#states[state] ?? this.#states[FALLBACK];
    if (spec === undefined) {
      throw new Error(`the pack has no "${FALLBACK}" state`);
    }
    return spec;
  }
}
