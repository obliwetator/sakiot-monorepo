import { API_ROUTES, apiUrl } from "../api/routes";
import {
	type ApiProblem,
	type ErrorKind,
	isTransient,
	MALFORMED_MESSAGE,
	problemFromError,
	problemFromResponse,
} from "./apiError";
import { authedFetch } from "./authedFetch";

export interface MediaJobStatus {
	id: string;
	kind: string;
	status: "queued" | "running" | "ready" | "failed";
	stage: string;
	progress: number;
	result_url?: string | null;
	/** Safe explanation of the last failed attempt. */
	error?: string | null;
	/** Absent on older servers and on records that predate it. */
	error_kind?: ErrorKind | null;
}

const POLL_INTERVAL_MS = 1_000;
const MAX_POLLS = 1_800;
/** Consecutive transient poll failures tolerated before giving up. */
const MAX_POLL_FAILURES = 5;

export const MEDIA_JOB_FAILED_MESSAGE = "Processing failed on the server.";

/** The server confirmed the job failed. */
export class MediaJobFailedError extends Error {
	readonly job: MediaJobStatus;

	constructor(job: MediaJobStatus) {
		super(job.error ?? MEDIA_JOB_FAILED_MESSAGE);
		this.name = "MediaJobFailedError";
		this.job = job;
	}
}

/**
 * Waiting stopped because the job's status could not be read. The server may
 * still be processing: this says nothing about the job itself.
 */
export class MediaJobUnreachableError extends Error {
	readonly problem: ApiProblem;

	constructor(problem: ApiProblem) {
		super(problem.message);
		this.name = "MediaJobUnreachableError";
		this.problem = problem;
	}
}

/** The job was still running when this page stopped waiting for it. */
export class MediaJobWaitTimeoutError extends Error {
	constructor() {
		super("Processing is taking longer than expected.");
		this.name = "MediaJobWaitTimeoutError";
	}
}

export function isMediaJobStatus(value: unknown): value is MediaJobStatus {
	if (typeof value !== "object" || value === null) return false;
	const job = value as Partial<Record<keyof MediaJobStatus, unknown>>;
	return (
		typeof job.id === "string" &&
		(job.status === "queued" ||
			job.status === "running" ||
			job.status === "ready" ||
			job.status === "failed")
	);
}

export function parseMediaJobStatus(value: unknown): MediaJobStatus | null {
	return isMediaJobStatus(value) ? value : null;
}

function delay(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted)
			return reject(new DOMException("Aborted", "AbortError"));
		const timer = globalThis.setTimeout(resolve, ms);
		signal?.addEventListener(
			"abort",
			() => {
				globalThis.clearTimeout(timer);
				reject(new DOMException("Aborted", "AbortError"));
			},
			{ once: true },
		);
	});
}

async function pollOnce(
	id: string,
	signal?: AbortSignal,
): Promise<MediaJobStatus | ApiProblem> {
	try {
		const response = await authedFetch(
			apiUrl(API_ROUTES.mediaJob, { job_id: id }),
			{ signal },
		);
		if (!response.ok) return problemFromResponse(response);
		const job = parseMediaJobStatus(await response.json().catch(() => null));
		return (
			job ?? {
				cause: "malformed",
				status: response.status,
				kind: null,
				message: MALFORMED_MESSAGE,
			}
		);
	} catch (error) {
		if (signal?.aborted) throw error;
		return problemFromError(error);
	}
}

/**
 * Poll a media job until it is ready. Rejects with {@link MediaJobFailedError}
 * only when the server reports failure; losing contact rejects with
 * {@link MediaJobUnreachableError} after a few transient retries.
 */
export async function waitForMediaJob(
	initial: MediaJobStatus,
	options: {
		signal?: AbortSignal;
		sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
	} = {},
): Promise<MediaJobStatus> {
	const { signal, sleep = delay } = options;
	let job = initial;
	let failures = 0;
	for (let poll = 0; poll < MAX_POLLS; poll += 1) {
		if (job.status === "ready") return job;
		if (job.status === "failed") throw new MediaJobFailedError(job);
		await sleep(POLL_INTERVAL_MS * 2 ** Math.min(failures, 3), signal);
		const next = await pollOnce(job.id, signal);
		if ("id" in next) {
			job = next;
			failures = 0;
			continue;
		}
		failures += 1;
		if (!isTransient(next) || failures >= MAX_POLL_FAILURES) {
			throw new MediaJobUnreachableError(next);
		}
	}
	throw new MediaJobWaitTimeoutError();
}
