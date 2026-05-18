import { useEffect } from "react";

export function useAppResume(onResume: () => void, enabled = true) {
  useEffect(() => {
    if (!enabled) return;

    const handleFocus = () => {
      onResume();
    };
    const handleVisible = () => {
      if (document.visibilityState === "visible") onResume();
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisible);

    return () => {
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisible);
    };
  }, [enabled, onResume]);
}
