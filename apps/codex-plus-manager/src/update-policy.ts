export type UpdatePromptInput = {
  updateAvailable?: boolean;
  latestVersion?: string | null;
};

export function shouldPromptForUpdate(
  update: UpdatePromptInput,
  ignoredVersion: string,
  promptedVersions: ReadonlySet<string>,
): boolean {
  const latestVersion = update.latestVersion?.trim() ?? "";
  return (
    update.updateAvailable === true &&
    latestVersion.length > 0 &&
    latestVersion !== ignoredVersion.trim() &&
    !promptedVersions.has(latestVersion)
  );
}
