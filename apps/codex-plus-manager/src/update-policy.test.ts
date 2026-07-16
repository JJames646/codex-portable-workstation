import assert from "node:assert/strict";
import test from "node:test";

import { shouldPromptForUpdate } from "./update-policy.ts";

test("available update prompts once per process", () => {
  const update = { updateAvailable: true, latestVersion: "v1.3.0" };

  assert.equal(shouldPromptForUpdate(update, "", new Set()), true);
  assert.equal(
    shouldPromptForUpdate(update, "", new Set(["v1.3.0"])),
    false,
  );
});

test("ignored version does not prompt", () => {
  assert.equal(
    shouldPromptForUpdate(
      { updateAvailable: true, latestVersion: "v1.3.0" },
      "v1.3.0",
      new Set(),
    ),
    false,
  );
});

test("newer version prompts after an older version was ignored", () => {
  assert.equal(
    shouldPromptForUpdate(
      { updateAvailable: true, latestVersion: "v1.4.0" },
      "v1.3.0",
      new Set(),
    ),
    true,
  );
});

test("missing update metadata never prompts", () => {
  assert.equal(
    shouldPromptForUpdate(
      { updateAvailable: false, latestVersion: "v1.3.0" },
      "",
      new Set(),
    ),
    false,
  );
  assert.equal(
    shouldPromptForUpdate(
      { updateAvailable: true, latestVersion: null },
      "",
      new Set(),
    ),
    false,
  );
});
