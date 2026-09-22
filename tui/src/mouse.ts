export type WheelDirection = -1 | 1;

export type WheelEvent = {
  scroll?: { direction?: string; delta?: number };
  button?: number;
};

export type WheelMemory = {
  at: number;
  direction: WheelDirection | null;
};

// Ghostty and other terminals often emit two scroll reports for one detent,
// a few milliseconds apart. A real extra notch is slower than this.
const DUPLICATE_WHEEL_MS = 40;

export function scrollDirection(event: WheelEvent): WheelDirection | null {
  if (event.scroll?.direction === "up" || event.button === 4) return -1;
  if (event.scroll?.direction === "down" || event.button === 5) return 1;
  return null;
}

export function acceptWheel(event: WheelEvent, now: number, previous: WheelMemory): { direction: WheelDirection | null; memory: WheelMemory } {
  const direction = scrollDirection(event);
  if (!direction) return { direction: null, memory: previous };
  if (previous.direction === direction && now - previous.at < DUPLICATE_WHEEL_MS) {
    return { direction: null, memory: { at: now, direction } };
  }
  return { direction, memory: { at: now, direction } };
}

let wheelMemory: WheelMemory = { at: Number.NEGATIVE_INFINITY, direction: null };

export function takeWheel(event: WheelEvent, now = Date.now()): WheelDirection | null {
  const accepted = acceptWheel(event, now, wheelMemory);
  wheelMemory = accepted.memory;
  return accepted.direction;
}

export function resetWheelMemory(): void {
  wheelMemory = { at: Number.NEGATIVE_INFINITY, direction: null };
}
