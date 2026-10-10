import { describe, expect, it } from "vitest";
import { credentialScope, hasScopedCredential } from "./credential-scope";

describe("credential destination binding", () => {
  it("normalizes origins without treating another port, host or provider as the same destination", () => {
    expect(credentialScope("api", "https://EXAMPLE.test:443/v1")).toBe("api|https://example.test");
    expect(credentialScope("api", "https://example.test:444/v1")).toBe("api|https://example.test:444");
    expect(credentialScope("cli", "https://example.test")).toBeNull();
    for (const url of ["http://example.test", "https://user:password@example.test", "https://example.test/#secret", "file:///tmp/x"]) expect(credentialScope("api", url)).toBeNull();
  });
  it("fails closed for old settings and mismatched saved task destinations", () => {
    expect(hasScopedCredential({ translation_provider: "api", base_url: "https://a.test" })).toBe(false);
    expect(hasScopedCredential({ translation_provider: "api", base_url: "https://a.test", api_key_scopes: ["api|https://b.test"] })).toBe(false);
    expect(hasScopedCredential({ translation_provider: "api", base_url: "https://a.test/v1", api_key_scopes: ["api|https://a.test"] })).toBe(true);
  });
});
