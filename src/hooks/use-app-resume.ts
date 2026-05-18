import { useEffect, useRef } from "react";

export function useAppResume(onResume: () => void, enabled = true) {
  const onResumeRef = useRef(onResume);

  useEffect(() => {
    onResumeRef.current = onResume;
  }, [onResume]);

  useEffect(() => {
    if (!enabled) return;

    const handleFocus = () => {
      onResumeRef.current();
    };
    const handleVisible = () => {
      if (document.visibilityState === "visible") onResumeRef.current();
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisible);

    return () => {
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisible);
    };
  }, [enabled]);
}
