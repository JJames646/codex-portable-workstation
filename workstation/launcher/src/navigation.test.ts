import { describe, expect, it } from "vitest";

import { resolveEngineSwitchPage } from "./navigation";

describe("resolveEngineSwitchPage", () => {
  it("keeps the Skill library open while switching installation targets", () => {
    expect(resolveEngineSwitchPage("skills", "claude")).toBe("skills");
    expect(resolveEngineSwitchPage("skills", "openclaw")).toBe("skills");
    expect(resolveEngineSwitchPage("skills", "codex")).toBe("skills");
  });

  it("keeps the tutorial page open while switching tutorial libraries", () => {
    expect(resolveEngineSwitchPage("tutorial", "claude")).toBe("tutorial");
    expect(resolveEngineSwitchPage("tutorial", "openclaw")).toBe("tutorial");
    expect(resolveEngineSwitchPage("tutorial", "codex")).toBe("tutorial");
  });

  it("keeps the plugin library only for Codex", () => {
    expect(resolveEngineSwitchPage("plugins", "codex")).toBe("plugins");
    expect(resolveEngineSwitchPage("plugins", "claude")).toBe("engine");
    expect(resolveEngineSwitchPage("plugins", "openclaw")).toBe("engine");
  });

  it("opens the selected engine configuration from other pages", () => {
    expect(resolveEngineSwitchPage("home", "claude")).toBe("engine");
    expect(resolveEngineSwitchPage("chat", "openclaw")).toBe("engine");
  });
});
