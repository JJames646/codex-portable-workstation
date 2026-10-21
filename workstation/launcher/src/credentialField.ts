export const SAVED_API_KEY_MASK = "••••••••••••••••";

export function isSavedCredentialMasked(
  draftValue: string,
  credentialPresent: boolean,
  editing: boolean,
): boolean {
  return credentialPresent && !editing && draftValue.length === 0;
}

export function credentialDisplayValue(
  draftValue: string,
  credentialPresent: boolean,
  editing: boolean,
): string {
  return isSavedCredentialMasked(draftValue, credentialPresent, editing)
    ? SAVED_API_KEY_MASK
    : draftValue;
}
