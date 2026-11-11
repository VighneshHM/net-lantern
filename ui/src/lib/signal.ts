export type SignalLabel = "Excellent" | "Good" | "Fair" | "Weak";

/** A plain-words label for a signal strength in dBm. */
export function signalLabel(dbm: number): SignalLabel {
  if (dbm >= -50) return "Excellent";
  if (dbm >= -60) return "Good";
  if (dbm >= -70) return "Fair";
  return "Weak";
}
