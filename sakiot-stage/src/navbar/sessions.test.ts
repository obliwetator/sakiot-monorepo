import { describe, expect, it } from "bun:test";
import { deviceKind, deviceLabel, lastActiveLabel } from "./sessions";

describe("signed-in devices", () => {
	it("names a device from whatever the server recognized", () => {
		expect(deviceLabel({ browser: "Chrome", platform: "Android" })).toBe(
			"Chrome on Android",
		);
		expect(deviceLabel({ browser: "Firefox", platform: null })).toBe("Firefox");
		expect(deviceLabel({ browser: null, platform: "iPhone" })).toBe("iPhone");
		expect(deviceLabel({})).toBe("Unknown device");
	});

	it("tells phones and tablets from computers", () => {
		expect(deviceKind("Android")).toBe("phone");
		expect(deviceKind("iPhone")).toBe("phone");
		expect(deviceKind("iPad")).toBe("tablet");
		expect(deviceKind("Windows")).toBe("computer");
		expect(deviceKind(null)).toBe("computer");
	});

	it("says how long ago a session was active", () => {
		const now = Date.parse("2026-10-10T12:00:00Z");
		const ago = (ms: number) => new Date(now - ms).toISOString();
		const minute = 60 * 1000;
		expect(lastActiveLabel(ago(5 * minute), now)).toBe("Active now");
		expect(lastActiveLabel(ago(40 * minute), now)).toBe(
			"Active 40 minutes ago",
		);
		expect(lastActiveLabel(ago(3 * 60 * minute), now)).toBe(
			"Active 3 hours ago",
		);
		expect(lastActiveLabel(ago(26 * 60 * minute), now)).toBe(
			"Active yesterday",
		);
		expect(lastActiveLabel("not a date", now)).toBe("Last active unknown");
	});
});
