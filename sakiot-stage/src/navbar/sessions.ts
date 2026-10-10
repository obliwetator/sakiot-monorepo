import type { SessionInfo } from "../app/apiSlice";

/** "Chrome on Android"; whichever half the server recognized otherwise. */
export function deviceLabel(
	session: Pick<SessionInfo, "browser" | "platform">,
): string {
	const { browser, platform } = session;
	if (browser && platform) return `${browser} on ${platform}`;
	return browser ?? platform ?? "Unknown device";
}

export type DeviceKind = "phone" | "tablet" | "computer";

export function deviceKind(platform: string | null | undefined): DeviceKind {
	if (platform === "iPhone" || platform === "Android") return "phone";
	if (platform === "iPad") return "tablet";
	return "computer";
}

/** An open tab refreshes its session at least every 15 minutes. */
const ACTIVE_NOW_MS = 15 * 60 * 1000;

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
	["year", 365 * 24 * 60 * 60 * 1000],
	["month", 30 * 24 * 60 * 60 * 1000],
	["day", 24 * 60 * 60 * 1000],
	["hour", 60 * 60 * 1000],
	["minute", 60 * 1000],
];

const relative = new Intl.RelativeTimeFormat("en", { numeric: "auto" });

/** "Active now", "Active 3 hours ago", "Active yesterday". */
export function lastActiveLabel(lastActiveAt: string, nowMs: number): string {
	const elapsed = nowMs - Date.parse(lastActiveAt);
	if (!Number.isFinite(elapsed)) return "Last active unknown";
	if (elapsed < ACTIVE_NOW_MS) return "Active now";
	for (const [unit, size] of UNITS) {
		if (elapsed >= size) {
			return `Active ${relative.format(-Math.floor(elapsed / size), unit)}`;
		}
	}
	return "Active now";
}
