import { describe, expect, it } from "vitest";
import { asrSetupSelectionKey, createAsrSetupPlan, matchesAsrSetupConsent, replanAsrSetup } from "./asr-setup-plan";
import { runtimeAcknowledgement, type AsrComponentStatus, type AsrModelComponent, type AsrRuntimeComponent } from "./asr-components";

const runtime: AsrRuntimeComponent = { id: "runtime", version: "1", label: "Runtime", platform: "windows-x64", engine: "qwen3-asr", backend: "qwen-asr", device: "cpu", license: "MIT", license_url: "https://example.test/license", installed_bytes: 200, max_files: 10, entrypoint: "python.exe", archive: { url: "https://example.test/runtime.zip", bytes: 100, sha256: "a".repeat(64) } };
const model: AsrModelComponent = { id: "model", version: "1", label: "Model", engine: runtime.engine, backend: runtime.backend, role: "model", license: "MIT", license_url: "https://example.test/license", installed_bytes: 500, files: [{ path: "model.bin", url: "https://example.test/model.bin", bytes: 500, sha256: "b".repeat(64) }] };
const aligner: AsrModelComponent = { ...model, id: "aligner", role: "aligner", files: [{ ...model.files[0], bytes: 200 }] };
const recipe: AsrRuntimeComponent = { ...runtime, archive: null, plan_sha256: "c".repeat(64), recipe: { schema: 1,
  python: { url: "https://example.test/python.zip", bytes: 100, sha256: "d".repeat(64), installed_bytes: 200, max_files: 10, entrypoint: "python.exe", site_packages: "Lib/site-packages", pip_version: "25" }, wheels: [],
  terms: [{ id: "terms", version: "1", sha256: "e".repeat(64), url: "https://example.test/terms", text: "The full required terms." }] } };
const base = { engine: runtime.engine, runtime, model, aligner, components: [] as AsrComponentStatus[], consents: [], verified: true };
function installed(id: string, patch: Partial<AsrComponentStatus> = {}): AsrComponentStatus { return { id, kind: id === "runtime" ? "runtime" : "model", state: "installed", version: "1", path: `/private/${id}`, python_path: id === "runtime" ? "/private/runtime/python" : null, installed_bytes: 100, error: null, ...patch }; }

describe("immutable composite ASR setup plan", () => {
  it("plans runtime, ASR model, and Qwen aligner in dependency order", () => {
    const plan = createAsrSetupPlan(base)!;
    expect(plan.steps.map((step) => [step.component.id, step.action])).toEqual([["runtime", "install"], ["model", "install"], ["aligner", "install"]]);
    expect(plan.downloadBytes).toBe(800); expect(plan.readyCount).toBe(0);
  });
  it("skips only exact backend-verified installations and counts only missing component bytes", () => {
    const plan = createAsrSetupPlan({ ...base, components: [installed("runtime"), installed("aligner")] })!;
    expect(plan.steps.map((step) => step.component.id)).toEqual(["model"]); expect(plan.downloadBytes).toBe(500); expect(plan.readyCount).toBe(2);
    expect(createAsrSetupPlan({ ...base, verified: false })).toBeNull();
    expect(createAsrSetupPlan({ ...base, components: [installed("runtime"), installed("model"), installed("aligner")] })?.steps).toEqual([]);
  });
  it("repairs damaged, mismatched, incomplete and invalid-path installations", () => {
    for (const patch of [{ state: "damaged" as const }, { version: "old" }, { version: null }, { python_path: null }, { python_path: "python" }, { error: "hash mismatch" }]) {
      const plan = createAsrSetupPlan({ ...base, components: [installed("runtime", patch)] })!;
      expect(plan.steps[0].action).toBe("repair");
    }
  });
  it("does not count arbitrary cache bytes as verified downloads", () => {
    const plan = createAsrSetupPlan({ ...base, components: [installed("runtime", { state: "not_installed", cached_bytes: 99_999 })] })!;
    expect(plan.steps[0].action).toBe("install"); expect(plan.downloadBytes).toBe(800);
  });
  it("fails closed on incomplete or incompatible selections", () => {
    for (const patch of [{ aligner: undefined }, { model: undefined }, { runtime: undefined }, { model: { ...model, backend: "other" } }, { model: { ...model, role: "aligner" as const } }, { aligner: { ...aligner, engine: "other" } }, { aligner: { ...aligner, id: "model" } }, { runtime: { ...runtime, unavailable_reason: "Unsupported" } }]) {
      expect(createAsrSetupPlan({ ...base, ...patch })).toBeNull();
    }
  });
  it("rejects missing or invalid download artifacts while permitting already verified installations", () => {
    expect(createAsrSetupPlan({ ...base, model: { ...model, files: [] } })).toBeNull();
    expect(createAsrSetupPlan({ ...base, runtime: { ...runtime, archive: null } })).toBeNull();
    for (const bytes of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(createAsrSetupPlan({ ...base, model: { ...model, files: [{ ...model.files[0], bytes }] } })).toBeNull();
    }
    expect(createAsrSetupPlan({ ...base, runtime: { ...runtime, archive: null }, components: [installed("runtime")] })?.readyCount).toBe(1);
    expect(createAsrSetupPlan({ ...base, model: { ...model, files: [] }, components: [installed("model")] })?.readyCount).toBe(1);
  });
  it("does not require an aligner for Whisper", () => {
    const plan = createAsrSetupPlan({ ...base, engine: "whisper-accelerated", runtime: { ...runtime, engine: "whisper-accelerated" }, model: { ...model, engine: "whisper-accelerated" } })!;
    expect(plan.selected).toHaveLength(2); expect(plan.aligner).toBeUndefined();
    expect(plan.key).toBe(asrSetupSelectionKey({ engine: plan.engine, runtime: plan.runtime, model: plan.model, aligner }));
  });
  it("takes deeply frozen copies with full metadata and term fingerprints", () => {
    const original = structuredClone(recipe);
    const plan = createAsrSetupPlan({ ...base, runtime: original })!;
    original.recipe!.terms[0].text = "Changed content";
    expect(plan.runtime.recipe!.terms[0].text).toBe("The full required terms.");
    expect(Object.isFrozen(plan.steps)).toBe(true); expect(Object.isFrozen(plan.runtime.recipe!.terms[0])).toBe(true);
    expect(plan.key).not.toBe(asrSetupSelectionKey({ ...base, runtime: original }));
    expect(() => { plan.runtime.recipe!.terms[0].text = "mutate"; }).toThrow();
  });
  it("requires exact complete runtime consent for an install even when a receipt exists", () => {
    const consent = runtimeAcknowledgement(recipe)!;
    const receipt = { component_id: runtime.id, plan_sha256: consent.plan_sha256, terms: consent.acknowledged_terms };
    expect(createAsrSetupPlan({ ...base, runtime: recipe, consents: [receipt] })?.steps[0]).toMatchObject({ consentRequired: true, consent });
    const damaged = { ...base, runtime: recipe, components: [installed("runtime", { state: "damaged" as const })] };
    expect(createAsrSetupPlan({ ...damaged, consents: [receipt] })?.steps[0].consentRequired).toBe(false);
    for (const bad of [{ ...receipt, plan_sha256: "f".repeat(64) }, { ...receipt, component_id: "other" }, { ...receipt, terms: [] }]) {
      expect(createAsrSetupPlan({ ...damaged, consents: [bad] })?.steps[0].consentRequired).toBe(true);
    }
    expect(matchesAsrSetupConsent(consent, { ...consent, acknowledged_terms: [] })).toBe(false);
  });
  it("rejects invalid or missing exact recipe terms before presenting an install plan", () => {
    expect(createAsrSetupPlan({ ...base, runtime: { ...recipe, plan_sha256: null } })).toBeNull();
    expect(createAsrSetupPlan({ ...base, runtime: { ...recipe, recipe: { ...recipe.recipe!, terms: [] } } })).toBeNull();
  });
  it("replans from current inventory but rejects catalog metadata or platform drift", () => {
    const plan = createAsrSetupPlan(base)!;
    const inventory = { catalog: { schema: 1 as const, platform: runtime.platform, runtimes: [runtime], models: [model, aligner] }, components: [installed("runtime")], consents: [] };
    expect(replanAsrSetup(plan, inventory)?.steps.map((step) => step.component.id)).toEqual(["model", "aligner"]);
    expect(replanAsrSetup(plan, { ...inventory, catalog: { ...inventory.catalog, platform: "other" } })).toBeNull();
    expect(replanAsrSetup(plan, { ...inventory, catalog: { ...inventory.catalog, models: [{ ...model, files: [{ ...model.files[0], url: "https://changed.test/model.bin" }] }, aligner] } })).toBeNull();
  });
});
