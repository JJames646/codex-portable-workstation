import { describe, expect, it } from "vitest";
import { formatBytes, progressPercent } from "./format";

describe("installer format helpers", () => {
  it("formats large payload", () => expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB"));
  it("calculates progress", () => expect(progressPercent(1, 4)).toBe(25));
});
