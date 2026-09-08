import type { RemoveSilenceResponse } from "../../../app/apiSlice";

/** The server answers the first call with this message while it spawns ffmpeg. */
export const SILENCE_JOB_PENDING_MESSAGE = "Request Accepted";
/** Gap between accepted responses. The follow-up call blocks server-side, so
 * a healthy job normally needs one retry and never reaches the gap. */
export const SILENCE_JOB_POLL_INTERVAL_MS = 1_000;
/** Hard bound on how long we keep asking. A server that keeps answering
 * "Request Accepted" must not spin the browser forever. */
export const SILENCE_JOB_MAX_ATTEMPTS = 60;

export class SilenceJobTimeoutError extends Error {
	constructor(attempts: number) {
		super(`Silence removal did not finish after ${attempts} attempts`);
		this.name = "SilenceJobTimeoutError";
	}
}

export class SilenceJobAbortedError extends Error {
	constructor() {
		super("Silence removal wait was aborted");
		this.name = "SilenceJobAbortedError";
	}
}

export interface SilenceJobPollOptions {
	/** One call to the remove-silence endpoint. Must reuse the same
	 * idempotency key on every call so the server treats it as one job. */
	request: () => Promise<RemoveSilenceResponse>;
	signal?: AbortSignal;
	intervalMs?: number;
	maxAttempts?: number;
	sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

function abortError(): Error {
	return typeof DOMException === "function"
		? new DOMException("Aborted", "AbortError")
		: new SilenceJobAbortedError();
}

function defaultSleep(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted) {
			reject(abortError());
			return;
		}
		const timer = setTimeout(() => {
			signal?.removeEventListener("abort", onAbort);
			resolve();
		}, ms);
		const onAbort = () => {
			clearTimeout(timer);
			reject(abortError());
		};
		signal?.addEventListener("abort", onAbort, { once: true });
	});
}

/**
 * Waits for a remove-silence job to leave the "Request Accepted" state.
 *
 * The first request starts the job and returns immediately; a later request
 * carrying the same idempotency key blocks until the job finishes. That means
 * one retry in practice — but the loop is bounded, spaced, and abortable so a
 * misbehaving server can neither hang the tab nor hammer the endpoint.
 */
export async function waitForSilenceJob(
	options: SilenceJobPollOptions,
): Promise<RemoveSilenceResponse> {
	const intervalMs = options.intervalMs ?? SILENCE_JOB_POLL_INTERVAL_MS;
	const maxAttempts = options.maxAttempts ?? SILENCE_JOB_MAX_ATTEMPTS;
	const sleep = options.sleep ?? defaultSleep;

	for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
		if (options.signal?.aborted) throw abortError();

		const response = await options.request();
		if (response.message !== SILENCE_JOB_PENDING_MESSAGE) return response;

		if (attempt === maxAttempts) throw new SilenceJobTimeoutError(attempt);
		await sleep(intervalMs, options.signal);
	}

	// Unreachable: the loop returns or throws on its final attempt.
	throw new SilenceJobTimeoutError(maxAttempts);
}
