import { describe, expect, it } from "vitest";

import {
  SAVED_API_KEY_MASK,
  credentialDisplayValue,
  isSavedCredentialMasked,
} from "./credentialField";

describe("saved provider credential display", () => {
  it("shows a UI-only mask when the vault reports an existing credential", () => {
    expect(isSavedCredentialMasked("", true, false)).toBe(true);
    expect(credentialDisplayValue("", true, false)).toBe(SAVED_API_KEY_MASK);
  });

  it("never replaces a newly typed credential with the saved mask", () => {
    expect(isSavedCredentialMasked("new-secret", true, true)).toBe(false);
    expect(credentialDisplayValue("new-secret", true, true)).toBe("new-secret");
  });

  it("uses an empty editable value without exposing the stored credential", () => {
    expect(isSavedCredentialMasked("", true, true)).toBe(false);
    expect(credentialDisplayValue("", true, true)).toBe("");
  });
});
