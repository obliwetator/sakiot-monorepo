import { describe, expect, test } from "bun:test";
import { JOB_POLL_WHILE_LIVE_MS, jobPollingInterval } from "./status";

describe("jobPollingInterval", () => {
	test("polls at the page's own rate without realtime", () => {
		expect(jobPollingInterval(1_000, false)).toBe(1_000);
		expect(jobPollingInterval(0, false)).toBe(0);
	});

	test("only backs up job events while realtime is live", () => {
		expect(jobPollingInterval(1_000, true)).toBe(JOB_POLL_WHILE_LIVE_MS);
		expect(jobPollingInterval(30_000, true)).toBe(30_000);
		// A page that is not waiting on a job does not start polling.
		expect(jobPollingInterval(0, true)).toBe(0);
	});
});
