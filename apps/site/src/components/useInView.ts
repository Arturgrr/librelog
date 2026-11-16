"use client";

import { type RefObject, useEffect, useState } from "react";

// "waiting" from hydration until the element is first seen, then "in" for good. Before hydration, and
// without JavaScript, it is "static", so content never stays hidden.
export function useInView(ref: RefObject<Element | null>, threshold = 0.15) {
  const [state, setState] = useState<"static" | "waiting" | "in">("static");

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setState("waiting");
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        setState("in");
        observer.disconnect();
      },
      { threshold },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, threshold]);

  return state;
}
