import type { Device } from "../api/queries";
import { formatAgo } from "./time";

/** The one-line presence text for a device, measured at `now` (unix ms). */
export function statusText(
  d: Pick<Device, "online" | "answers_ping" | "last_seen">,
  now: number,
): string {
  if (!d.online) return `Offline · last seen ${formatAgo(d.last_seen, now)}`;
  return d.answers_ping ? "Online" : "Online · doesn't answer ping";
}
