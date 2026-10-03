import { describe, expect, it } from "bun:test";
import {
	MEMBERSHIP_RETRIES,
	membershipRetryDelay,
	retryWhileMembershipLoads,
} from "./membershipRetry";

const loading = (retry_after_seconds?: unknown) => ({
	error: {
		status: 503,
		data: {
			code: 503,
			kind: "membership_unavailable",
			message: "This server's member list is still loading.",
			retry_after_seconds,
		},
	},
});
const ok = { data: "guilds" };

describe("membershipRetryDelay", () => {
	it("uses the server's delay, bounded, only for membership_unavailable", () => {
		expect(membershipRetryDelay(loading(5))).toBe(5);
		expect(membershipRetryDelay(loading(600))).toBe(10);
		expect(membershipRetryDelay(loading())).toBe(10);
		expect(membershipRetryDelay(loading("soon"))).toBe(10);
		expect(membershipRetryDelay(ok)).toBeNull();
		expect(
			membershipRetryDelay({
				error: { status: 503, data: { kind: "bot_unavailable" } },
			}),
		).toBeNull();
		expect(
			membershipRetryDelay({ error: { status: 503, data: "<html>" } }),
		).toBeNull();
	});
});

describe("retryWhileMembershipLoads", () => {
	it("repeats a read until the member list has loaded", async () => {
		const answers = [loading(5), loading(5), ok];
		const waits: number[] = [];
		const result = await retryWhileMembershipLoads(
			async () => answers.shift() ?? ok,
			new AbortController().signal,
			async (ms) => {
				waits.push(ms);
			},
		);
		expect(result).toEqual(ok);
		expect(waits).toEqual([5000, 5000]);
	});

	it("gives up after a bounded number of attempts", async () => {
		let calls = 0;
		const result = await retryWhileMembershipLoads(
			async () => {
				calls++;
				return loading(5);
			},
			new AbortController().signal,
			async () => {},
		);
		expect(result).toEqual(loading(5));
		expect(calls).toBe(MEMBERSHIP_RETRIES + 1);
	});

	it("stops waiting when the request is aborted", async () => {
		const controller = new AbortController();
		let calls = 0;
		const pending = retryWhileMembershipLoads(async () => {
			calls++;
			return loading(5);
		}, controller.signal);
		await Promise.resolve();
		controller.abort();
		expect(await pending).toEqual(loading(5));
		expect(calls).toBe(1);
	});
});
