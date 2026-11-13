import { useEffect, useState } from "react";

/** The current time in ms, refreshed every 30 s so "5 min ago" text stays true. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);
  return now;
}
