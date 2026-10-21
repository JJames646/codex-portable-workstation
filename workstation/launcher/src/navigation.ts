export type PageId =
  | "home"
  | "engine"
  | "chat"
  | "image"
  | "skills"
  | "plugins"
  | "tutorial"
  | "settings";

export type EngineId = "codex" | "claude" | "openclaw";

export function resolveEngineSwitchPage(
  currentPage: PageId,
  nextEngine: EngineId,
): PageId {
  if (currentPage === "skills" || currentPage === "tutorial") {
    return currentPage;
  }

  if (currentPage === "plugins") {
    return nextEngine === "codex" ? "plugins" : "engine";
  }

  return "engine";
}
