import type { SettingsState } from "@/types";

export function credentialScope(provider: string, baseUrl: string): string | null {
  if (provider.trim().toLowerCase() !== "api") return null;
  try {
    const url = new URL(baseUrl.trim());
    const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    if (url.username || url.password || url.hash || (url.protocol !== "https:" && !(url.protocol === "http:" && loopback))) return null;
    return `api|${url.origin}`;
  } catch { return null; }
}

export function hasScopedCredential(settings: Pick<SettingsState, "translation_provider" | "base_url" | "api_key_scopes">): boolean {
  const scope = credentialScope(settings.translation_provider, settings.base_url);
  return scope !== null && (settings.api_key_scopes ?? []).includes(scope);
}
