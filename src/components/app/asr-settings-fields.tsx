import { useId, type ReactNode } from "react";
import { AlertCircle, CheckCircle2, Loader2, ScanLine, Trash2 } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import optionalAsrGuide from "../../../docs/OPTIONAL_ASR.md?raw";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useAsrBackendCheck } from "@/hooks/use-asr-backend-check";
import { asrConfigurationIssues, asrEngines, isAbsoluteLocalPath, normalizeAsrConfig } from "@/lib/asr-config";
import type { AsrConfig, TFunction } from "@/types";

function storageLabel(bytes: number) {
  return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
}

export function AsrSettingsFields({ value, onChange, disabled = false, t, children }: {
  value?: AsrConfig;
  onChange: (config: AsrConfig) => void;
  disabled?: boolean;
  t: TFunction;
  children: ReactNode;
}) {
  const id = useId();
  const config = normalizeAsrConfig(value);
  const isLegacy = config.engine === "whisper-cpp";
  const isQwen = config.engine === "qwen3-asr";
  const knownEngine = asrEngines.some((engine) => engine === config.engine);
  const issues = asrConfigurationIssues(config);
  const { checking, releasing, result, error, notice, check, release } = useAsrBackendCheck(config, t);
  const busy = checking || releasing;
  const change = (next: Partial<AsrConfig>) => onChange({ ...config, ...next });
  const pathFields = [
    { name: "python_path", label: "asr.pythonPath", hint: "asr.pythonHint" },
    { name: "model_path", label: "asr.modelPath", hint: isQwen ? "asr.qwenModelHint" : "asr.whisperModelHint" },
    ...(isQwen ? [{ name: "aligner_path", label: "asr.alignerPath", hint: "asr.alignerHint" }] : []),
  ] as { name: "python_path" | "model_path" | "aligner_path"; label: string; hint: string }[];

  return (
    <FieldGroup>
      <Field data-disabled={disabled} data-invalid={!knownEngine}>
        <FieldLabel htmlFor={`${id}-engine`}>{t("asr.engine")}</FieldLabel>
        <Select value={config.engine} onValueChange={(engine) => change({ engine })} disabled={disabled}>
          <SelectTrigger id={`${id}-engine`} className="w-full" aria-invalid={!knownEngine}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="whisper-cpp">{t("asr.whisperCpp")}</SelectItem>
              <SelectItem value="whisper-accelerated">{t("asr.whisperAccelerated")}</SelectItem>
              <SelectItem value="qwen3-asr">{t("asr.qwen")}</SelectItem>
              {!knownEngine && <SelectItem value={config.engine || "unknown"}>{config.engine || t("asr.unknown")}</SelectItem>}
            </SelectGroup>
          </SelectContent>
        </Select>
        <FieldDescription>{t("asr.engineHint")}</FieldDescription>
      </Field>
      {isLegacy ? children : knownEngine ? (
        <>
          <Alert>
            <AlertCircle />
            <AlertTitle>{t("asr.optionalSetup")}</AlertTitle>
            <AlertDescription className="flex flex-col gap-2">
              <p>{t("asr.offlineSetup")}</p>
              <p>{t(isQwen ? "asr.qwenSetup" : "asr.whisperSetup")}</p>
              {isQwen && <p>{t("asr.qwenMac")}</p>}
              {isQwen && config.device !== "cuda" && <p>{t("asr.qwenCpuMemory")}</p>}
              <p>{t("asr.noBenchmark")}</p>
            </AlertDescription>
          </Alert>
          {pathFields.map(({ name, label, hint }) => {
            const invalid = !isAbsoluteLocalPath(config[name]);
            return (
              <Field key={name} data-disabled={disabled} data-invalid={invalid}>
                <FieldLabel htmlFor={`${id}-${name}`}>{t(label)}</FieldLabel>
                <Input
                  id={`${id}-${name}`}
                  value={config[name]}
                  onChange={(event) => change({ [name]: event.target.value })}
                  disabled={disabled}
                  aria-invalid={invalid}
                  aria-describedby={`${id}-${name}-hint`}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={t("asr.absolutePath")}
                />
                <FieldDescription id={`${id}-${name}-hint`}>{t(hint)}</FieldDescription>
              </Field>
            );
          })}
          <Field data-disabled={disabled} data-invalid={issues.includes("unsupportedAsrDevice")}>
            <FieldLabel htmlFor={`${id}-device`}>{t("asr.device")}</FieldLabel>
            <Select value={config.device} onValueChange={(device) => change({ device: device as AsrConfig["device"] })} disabled={disabled}>
              <SelectTrigger id={`${id}-device`} className="w-full" aria-invalid={issues.includes("unsupportedAsrDevice")}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="auto">{t("asr.auto")}</SelectItem>
                  <SelectItem value="cpu">CPU</SelectItem>
                  <SelectItem value="cuda">CUDA</SelectItem>
                  <SelectItem value="metal" disabled={isQwen}>Metal (MLX)</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
            <FieldDescription>{t(isQwen ? "asr.qwenDeviceHint" : "asr.whisperDeviceHint")}</FieldDescription>
          </Field>
          <FieldDescription>
            {t(isQwen ? "asr.qwenStorage" : "asr.whisperStorage")}{" "}{t("asr.storageExtra")}
          </FieldDescription>
          <FieldDescription>
            {t("asr.officialSources")}{" "}
            {isQwen ? (
              <>
                <a href="https://github.com/QwenLM/Qwen3-ASR" target="_blank" rel="noreferrer">Qwen3-ASR</a>{" · "}
                <a href="https://huggingface.co/Qwen/Qwen3-ASR-0.6B/tree/main" target="_blank" rel="noreferrer">0.6B</a>{" · "}
                <a href="https://huggingface.co/Qwen/Qwen3-ASR-1.7B/tree/main" target="_blank" rel="noreferrer">1.7B</a>{" · "}
                <a href="https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B/tree/main" target="_blank" rel="noreferrer">ForcedAligner</a>
              </>
            ) : (
              <>
                <a href="https://github.com/SYSTRAN/faster-whisper" target="_blank" rel="noreferrer">faster-whisper</a>{" · "}
                <a href="https://github.com/ml-explore/mlx-examples/tree/main/whisper" target="_blank" rel="noreferrer">MLX Whisper</a>
              </>
            )}
          </FieldDescription>
          <details>
            <summary className="cursor-pointer">{t("asr.setupGuide")}</summary>
            <FieldDescription>{t("asr.setupGuideHint")}</FieldDescription>
            <ScrollArea className="h-80">
              <pre className="whitespace-pre-wrap break-words p-3 text-sm">{optionalAsrGuide}</pre>
            </ScrollArea>
          </details>
          <div className="flex flex-wrap gap-2">
            <Button type="button" variant="secondary" disabled={disabled || busy || issues.length > 0} onClick={check}>
              {checking ? <Loader2 data-icon="inline-start" className="spin" /> : <ScanLine data-icon="inline-start" />}
              {t(checking ? "asr.checking" : "asr.check")}
            </Button>
          </div>
          {issues.length > 0 && <FieldDescription role="alert">{issues.map((issue) => t(`requirement.${issue}`)).join(t("requirement.separator"))}</FieldDescription>}
          <div aria-live="polite">
            {result && (
              <Alert variant={result.ready ? "default" : "destructive"}>
                {result.ready ? <CheckCircle2 /> : <AlertCircle />}
                <AlertTitle>{t(result.ready ? "asr.ready" : "asr.notReady")}</AlertTitle>
                <AlertDescription className="flex flex-col gap-2">
                  <p>{t("asr.detected", { backend: result.backend ?? t("asr.unknown"), device: result.device ?? t("asr.unknown") })}</p>
                  <p>{t("asr.localStorage", { model: storageLabel(result.model_bytes), aligner: storageLabel(result.aligner_bytes), total: storageLabel(result.total_bytes) })}</p>
                  <p>{t(result.capabilities.word_timestamps ? "asr.wordTimestamps" : "asr.noWordTimestamps")}</p>
                  {result.capabilities.supported_languages && <p>{t("asr.languages", { languages: result.capabilities.supported_languages.join(", ") })}</p>}
                  {result.error && <p>{result.code ? `${result.code}: ` : ""}{result.error}</p>}
                  {result.warnings.map((warning, index) => <p key={`${index}-${warning}`}>{warning}</p>)}
                  {!result.ready && <p>{t("asr.recovery")}</p>}
                  {result.ready && <p>{t("asr.checkScope")}</p>}
                </AlertDescription>
              </Alert>
            )}
          </div>
        </>
      ) : (
        <Alert variant="destructive"><AlertCircle /><AlertTitle>{t("requirement.unsupportedAsrEngine")}</AlertTitle><AlertDescription>{t("asr.unknownRecovery")}</AlertDescription></Alert>
      )}
      <div className="flex flex-col items-start gap-2">
        <Button type="button" variant="outline" disabled={disabled || busy} onClick={release}>
          {releasing ? <Loader2 data-icon="inline-start" className="spin" /> : <Trash2 data-icon="inline-start" />}
          {t("asr.release")}
        </Button>
        <FieldDescription>{t("asr.warmMemory")}</FieldDescription>
      </div>
      <div aria-live="polite">
        {error && <Alert variant="destructive"><AlertCircle /><AlertTitle>{t("asr.checkFailed")}</AlertTitle><AlertDescription>{error} {t("asr.recovery")}</AlertDescription></Alert>}
        {notice && <p>{notice}</p>}
      </div>
    </FieldGroup>
  );
}
