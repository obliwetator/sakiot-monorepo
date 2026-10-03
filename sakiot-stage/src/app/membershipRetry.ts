/**
 * Membership comes from the bot's member list. While the bot is still
 * loading a guild's list (just after it joins, or on its first start) the
 * server cannot say who belongs and answers reads with a 503
 * `membership_unavailable` and a retry delay rather than a guess. Reads wait
 * and repeat for a while instead of showing an error; writes are not
 * repeated, and their error says to try again.
 */

/** How many times one read is repeated before its error is shown. */
export const MEMBERSHIP_RETRIES = 6;
/** Fallback and upper bound for the server's retry delay, in seconds. */
const MAX_DELAY_SECONDS = 10;

interface QueryOutcome {
	error?: { status: unknown; data?: unknown };
}

/** The retry delay of a `membership_unavailable` answer, else null. */
export function membershipRetryDelay(result: QueryOutcome): number | null {
	const error = result.error;
	if (!error || error.status !== 503) return null;
	const data = error.data;
	if (typeof data !== "object" || data === null) return null;
	if (!("kind" in data) || data.kind !== "membership_unavailable") return null;
	const seconds =
		"retry_after_seconds" in data ? data.retry_after_seconds : undefined;
	return typeof seconds === "number" && Number.isFinite(seconds) && seconds > 0
		? Math.min(seconds, MAX_DELAY_SECONDS)
		: MAX_DELAY_SECONDS;
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		if (signal.aborted) return resolve();
		const timer = setTimeout(done, ms);
		function done() {
			clearTimeout(timer);
			signal.removeEventListener("abort", done);
			resolve();
		}
		signal.addEventListener("abort", done);
	});
}

/**
 * Runs `request` and repeats it while the server answers
 * `membership_unavailable`, up to `MEMBERSHIP_RETRIES` times. Stops early,
 * returning the last answer, when `signal` aborts.
 */
export async function retryWhileMembershipLoads<T extends QueryOutcome>(
	request: () => T | PromiseLike<T>,
	signal: AbortSignal,
	wait: (ms: number, signal: AbortSignal) => Promise<void> = delay,
): Promise<T> {
	let result = await request();
	for (let attempt = 0; attempt < MEMBERSHIP_RETRIES; attempt++) {
		const seconds = membershipRetryDelay(result);
		if (seconds === null) return result;
		await wait(seconds * 1000, signal);
		if (signal.aborted) return result;
		result = await request();
	}
	return result;
}
