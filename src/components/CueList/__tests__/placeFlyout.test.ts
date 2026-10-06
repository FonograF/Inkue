import { describe, expect, it } from "vitest";

import { placeFlyout } from "../ContextMenuParts";

const viewport = { width: 1600, height: 800 };
const flyout = { width: 180, height: 200 };
const rowAt = (left: number, top: number) => ({ left, right: left + 220, top, width: 220, height: 27 });

describe("placeFlyout", () => {
  it("opens beside the row on the right, aligned with it", () => {
    expect(placeFlyout(rowAt(400, 300), flyout, false, viewport)).toEqual({ left: 620, top: 295 });
  });

  it("opens on the left when asked to", () => {
    expect(placeFlyout(rowAt(400, 300), flyout, true, viewport).left).toBe(220);
  });

  it("flips to the left when the right side has no room", () => {
    expect(placeFlyout(rowAt(1300, 300), flyout, false, viewport).left).toBe(1120);
  });

  it("stays on the right when the left side has no room either way", () => {
    expect(placeFlyout(rowAt(50, 300), flyout, true, viewport).left).toBe(270);
  });

  it("is pulled up to stay inside the bottom of the window", () => {
    expect(placeFlyout(rowAt(400, 750), flyout, false, viewport).top).toBe(596);
  });

  it("never starts above the window", () => {
    expect(placeFlyout(rowAt(400, 0), { width: 180, height: 900 }, false, viewport).top).toBe(4);
  });
});
