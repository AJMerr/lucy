// The sprite's page: asks for the pack, then draws whatever frame the animator says, every
// display frame.

import { invoke } from "@tauri-apps/api/core";

import { Animator, type States } from "./animator.ts";

/** The pack as `lucy-sprite` hands it over (see src-tauri/src/pack.rs). */
interface PackView {
  name: string;
  width: number;
  height: number;
  scale: number;
  anchor: string;
  states: Record<string, { frames: string[]; fps: number; loop: boolean; then?: string }>;
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("a frame wouldn't load"));
    image.src = src;
  });
}

async function start(): Promise<void> {
  const pack = await invoke<PackView>("pack");
  const frames: Record<string, HTMLImageElement[]> = {};
  const timing: States = {};
  for (const [name, state] of Object.entries(pack.states)) {
    frames[name] = await Promise.all(state.frames.map(loadImage));
    timing[name] = {
      frames: state.frames.length,
      fps: state.fps,
      loop: state.loop,
      ...(state.then === undefined ? {} : { then: state.then }),
    };
  }

  const canvas = document.getElementById("sprite") as HTMLCanvasElement;
  const width = Math.round(pack.width * pack.scale);
  const height = Math.round(pack.height * pack.scale);
  const ratio = window.devicePixelRatio || 1;
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.width = Math.round(width * ratio);
  canvas.height = Math.round(height * ratio);
  const context = canvas.getContext("2d");
  if (context === null) {
    throw new Error("no 2D canvas");
  }
  // Pixel art stays crisp when scaled up.
  context.imageSmoothingEnabled = pack.scale * ratio < 1;

  const animator = new Animator(timing, performance.now());
  canvas.addEventListener("click", () => animator.play("happy", performance.now()));

  const draw = (now: number) => {
    const shown = animator.frame(now);
    const image = frames[shown.state]?.[shown.index];
    context.clearRect(0, 0, canvas.width, canvas.height);
    if (image !== undefined) {
      context.drawImage(image, 0, 0, canvas.width, canvas.height);
    }
    requestAnimationFrame(draw);
  };
  requestAnimationFrame(draw);
}

start().catch((err: unknown) => console.error("lucy-sprite:", err));
