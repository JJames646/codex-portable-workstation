import { describe, expect, it } from "vitest";
import { pluginInitials, safeBrandColor } from "./PluginCenter";

describe("PluginCenter helpers", () => {
  it("creates readable Chinese and Latin plugin marks", () => {
    expect(pluginInitials("Google Calendar")).toBe("GC");
    expect(pluginInitials("知识库")).toBe("知识");
  });

  it("rejects unsafe CSS brand values", () => {
    expect(safeBrandColor("#EA4335")).toBe("#EA4335");
    expect(safeBrandColor("url(javascript:alert(1))")).toBe("#356B5B");
  });
});
