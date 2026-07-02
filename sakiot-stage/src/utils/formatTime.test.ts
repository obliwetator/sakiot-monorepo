import { describe, expect, it } from "bun:test";
import {
	formatBytes,
	formatDuration,
	formatTimeSince,
	formatUptime,
} from "./formatTime";

describe("formatTime utilities", () => {
	it("formats durations as hh:mm:ss and guards invalid input", () => {
		expect(formatDuration(3661.9)).toBe("01:01:01");
		expect(formatDuration(Number.NaN)).toBe("00:00:00");
		expect(formatDuration(Number.POSITIVE_INFINITY)).toBe("00:00:00");
	});

	it("formats uptime and time-since text", () => {
		expect(formatUptime(90061)).toBe("1 day, 1 hour, 1 minute, 1 second");
		expect(formatUptime(0)).toBe("0 seconds");
		expect(formatTimeSince(1_000, 10)).toBe("9 seconds ago");
		expect(formatTimeSince(undefined, 10)).toBe("Never");
	});

	it("formats byte counts using MB or GB", () => {
		expect(formatBytes(2 * 1024 * 1024)).toBe("2.0 MB");
		expect(formatBytes(3 * 1024 * 1024 * 1024)).toBe("3.0 GB");
	});
});
