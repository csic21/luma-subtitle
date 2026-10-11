export function componentStorageLabel(bytes: number) {
  if (bytes === 0) return "0 MB";
  if (bytes < 1_000_000) return `${Math.ceil(bytes / 1_000)} KB`;
  if (bytes < 1_000_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
}
