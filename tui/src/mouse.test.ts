import { describe, expect, test } from "bun:test";
import { acceptWheel } from "./mouse";

describe("mouse wheel", () => {
  test("keeps one entry per detent when the terminal repeats the scroll report", () => {
    const first = acceptWheel({ scroll: { direction: "down", delta: 1 } }, 1_000, { at: 0, direction: null });
    expect(first.direction).toBe(1);
    const repeat = acceptWheel({ scroll: { direction: "down", delta: 1 } }, 1_008, first.memory);
    expect(repeat.direction).toBeNull();
    const next = acceptWheel({ scroll: { direction: "down", delta: 1 } }, 1_080, repeat.memory);
    expect(next.direction).toBe(1);
  });

  test("accepts the opposite direction immediately", () => {
    const down = acceptWheel({ scroll: { direction: "down" } }, 2_000, { at: 0, direction: null });
    const up = acceptWheel({ button: 4 }, 2_004, down.memory);
    expect(up.direction).toBe(-1);
  });
});
