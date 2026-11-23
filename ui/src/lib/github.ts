/**
 * Every update link the server sends starts with this. Keep it equal to
 * RELEASES_PAGE_PREFIX in crates/netlantern-core/src/updates.rs (the desktop
 * window's allow-list uses the same constant).
 */
export const RELEASES_PAGE_PREFIX = "https://github.com/VighneshHM/net-lantern/releases/";

/** True only for a Net Lantern release page on GitHub. */
export function isReleaseUrl(url: string | null | undefined): url is string {
  return typeof url === "string" && url.startsWith(RELEASES_PAGE_PREFIX);
}
