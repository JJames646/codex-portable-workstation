import { describe, expect, it } from "vitest";
import { formatBytes, progressPercent } from "./format";

describe("format helpers", () => {
  it("formats bytes", () => expect(formatBytes(1024 ** 3)).toBe("1.0 GB"));
  it("clamps progress", () => expect(progressPercent(120, 100)).toBe(100));
});
