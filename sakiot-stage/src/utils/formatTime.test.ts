import { describe, expect, it } from "bun:test";
import {
	formatBytes,
	formatDuration,
	formatSessionTimecode,
	formatTimeSince,
	formatUptime,
	parseSessionTimecode,
} from "./formatTime";

describe("formatTime utilities", () => {
	it("formats durations as hh:mm:ss and guards invalid input", () => {
		expect(formatDuration(3661.9)).toBe("01:01:01");
		expect(formatDuration(Number.NaN)).toBe("00:00:00");
		expect(formatDuration(Number.POSITIVE_INFINITY)).toBe("00:00:00");
	});

	it("formats session timecodes according to recording length", () => {
		expect(formatSessionTimecode(65, 3_599)).toBe("01:05");
		expect(formatSessionTimecode(65, 3_600)).toBe("00:01:05");
		expect(formatSessionTimecode(90_061, 100_000)).toBe("25:01:01");
		expect(formatSessionTimecode(Number.NaN, 3_600)).toBe("00:00:00");
	});

	it("parses duration-aware session timecodes", () => {
		expect(parseSessionTimecode("1:05", 3_599)).toBe(65);
		expect(parseSessionTimecode("1:01:05", 10_000)).toBe(3_665);
		expect(parseSessionTimecode("00:60", 3_599)).toBeNull();
		expect(parseSessionTimecode("01:00", 30)).toBeNull();
		expect(parseSessionTimecode("01:05", 3_600)).toBeNull();
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
