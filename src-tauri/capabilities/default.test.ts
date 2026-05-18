import { describe, expect, it } from "vitest";

import capability from "./default.json";

describe("default Tauri capability", () => {
  it("allows core event listeners used by task pages", () => {
    expect(capability.permissions).toContain("core:event:default");
  });
});
