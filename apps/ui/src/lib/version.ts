// This app's version (bumped with each release), and spotting a core that's older than it:
// after an upgrade, a core that was already running keeps the old version until it restarts.

export const VERSION = "1.2.0";

/** `core` is an older version than `mine` (false when either can't be read). */
export function olderCore(core: string, mine: string): boolean {
  const parse = (v: string) => {
    const parts = v.replace(/^v/, "").split(".").map(Number);
    return parts.every((n) => Number.isInteger(n) && n >= 0) ? parts : null;
  };
  const a = parse(core);
  const b = parse(mine);
  if (a === null || b === null) {
    return false;
  }
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) {
      return x < y;
    }
  }
  return false;
}
