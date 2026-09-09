import { describe, expect, test } from "bun:test";
import {
	CLIP_WAVEFORM_MAX_POLL_ERRORS,
	nextPollErrorCount,
	shouldKeepPollingClipWaveform,
} from "./clipWaveformPolling";

describe("shouldKeepPollingClipWaveform", () => {
	test("keeps polling while the build is incomplete", () => {
		expect(shouldKeepPollingClipWaveform(undefined, 0)).toBe(true);
		expect(shouldKeepPollingClipWaveform(0, 0)).toBe(true);
		expect(shouldKeepPollingClipWaveform(99, 0)).toBe(true);
	});

	test("stops once the server reports completion", () => {
		expect(shouldKeepPollingClipWaveform(100, 0)).toBe(false);
		expect(shouldKeepPollingClipWaveform(100, 4)).toBe(false);
	});

	test("retries transient failures instead of giving up immediately", () => {
		for (let errors = 1; errors < CLIP_WAVEFORM_MAX_POLL_ERRORS; errors += 1) {
			expect(shouldKeepPollingClipWaveform(40, errors)).toBe(true);
		}
	});

	test("gives up after a run of failures", () => {
		expect(
			shouldKeepPollingClipWaveform(40, CLIP_WAVEFORM_MAX_POLL_ERRORS),
		).toBe(false);
		expect(
			shouldKeepPollingClipWaveform(40, CLIP_WAVEFORM_MAX_POLL_ERRORS + 3),
		).toBe(false);
	});
});

describe("nextPollErrorCount", () => {
	test("only counts settled requests", () => {
		// A refetch clears isError while it is in flight; that must not reset
		// the streak or the cap would never be reached.
		expect(nextPollErrorCount(false, false, 3)).toBe(3);
		expect(nextPollErrorCount(false, true, 3)).toBe(3);
	});

	test("increments on failure and resets on success", () => {
		expect(nextPollErrorCount(true, true, 0)).toBe(1);
		expect(nextPollErrorCount(true, true, 1)).toBe(2);
		expect(nextPollErrorCount(true, false, 2)).toBe(0);
	});

	test("reaches the cap after five settled failures", () => {
		let streak = 0;
		for (
			let attempt = 0;
			attempt < CLIP_WAVEFORM_MAX_POLL_ERRORS;
			attempt += 1
		) {
			streak = nextPollErrorCount(true, true, streak);
		}
		expect(streak).toBe(CLIP_WAVEFORM_MAX_POLL_ERRORS);
		expect(shouldKeepPollingClipWaveform(40, streak)).toBe(false);
	});
});

describe("shouldKeepPollingClipWaveform server errors", () => {
	test("stops on a server-reported build error", () => {
		expect(shouldKeepPollingClipWaveform(40, 0, true)).toBe(false);
	});

	test("keeps polling without a server error", () => {
		expect(shouldKeepPollingClipWaveform(40, 0, false)).toBe(true);
	});
});
