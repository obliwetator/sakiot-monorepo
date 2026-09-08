import { describe, expect, it } from "bun:test";
import {
	SILENCE_JOB_PENDING_MESSAGE,
	SilenceJobTimeoutError,
	waitForSilenceJob,
} from "./silenceJobPoll";

function accepted() {
	return { message: SILENCE_JOB_PENDING_MESSAGE, url: "" };
}

function done(message = "Success") {
	return { message, url: "/no_silence.ogg" };
}

const noSleep = () => Promise.resolve();

describe("waitForSilenceJob", () => {
	it("returns the first non-pending response without sleeping", async () => {
		const calls: number[] = [];
		let slept = 0;
		const result = await waitForSilenceJob({
			request: async () => {
				calls.push(calls.length + 1);
				return done("File already exists");
			},
			sleep: async () => {
				slept += 1;
			},
		});

		expect(result.message).toBe("File already exists");
		expect(calls).toHaveLength(1);
		expect(slept).toBe(0);
	});

	it("retries after an accepted response and returns the outcome", async () => {
		let attempt = 0;
		const slept: number[] = [];
		const result = await waitForSilenceJob({
			request: async () => {
				attempt += 1;
				return attempt === 1 ? accepted() : done();
			},
			intervalMs: 250,
			sleep: async (ms) => {
				slept.push(ms);
			},
		});

		expect(result.message).toBe("Success");
		expect(attempt).toBe(2);
		expect(slept).toEqual([250]);
	});

	it("throws instead of looping forever when the server keeps accepting", async () => {
		let attempt = 0;
		await expect(
			waitForSilenceJob({
				request: async () => {
					attempt += 1;
					return accepted();
				},
				maxAttempts: 3,
				sleep: noSleep,
			}),
		).rejects.toBeInstanceOf(SilenceJobTimeoutError);
		expect(attempt).toBe(3);
	});

	it("does not sleep after the final attempt", async () => {
		let slept = 0;
		await waitForSilenceJob({
			request: async () => accepted(),
			maxAttempts: 2,
			sleep: async () => {
				slept += 1;
			},
		}).catch(() => {});
		expect(slept).toBe(1);
	});

	it("stops without requesting when already aborted", async () => {
		const controller = new AbortController();
		controller.abort();
		let requested = 0;
		await expect(
			waitForSilenceJob({
				request: async () => {
					requested += 1;
					return accepted();
				},
				signal: controller.signal,
				sleep: noSleep,
			}),
		).rejects.toHaveProperty("name", "AbortError");
		expect(requested).toBe(0);
	});

	it("stops between attempts when aborted mid-wait", async () => {
		const controller = new AbortController();
		let requested = 0;
		await expect(
			waitForSilenceJob({
				request: async () => {
					requested += 1;
					return accepted();
				},
				signal: controller.signal,
				sleep: async () => {
					controller.abort();
				},
			}),
		).rejects.toHaveProperty("name", "AbortError");
		expect(requested).toBe(1);
	});
});
