/**
 * Turns any failed request into a reason a person can act on.
 *
 * The server's error contract (`{ code, kind, message }`) is preferred: its
 * message is written for users and never carries internal detail. Everything
 * else — no response at all, an empty or non-JSON body, a proxy error page, a
 * body from an older server without `kind` — gets a generic reason derived
 * from what is known. Exception text and error objects are never displayed.
 *
 * A problem's message is only the reason: screens say what failed ("The clip
 * was not created.") and append it.
 */
import type { SerializedError } from "@reduxjs/toolkit";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { components } from "../api/openapi";
import { SESSION_EXPIRED_MESSAGE } from "./authedFetch";

export type ErrorKind = components["schemas"]["ErrorKind"];

export type ProblemCause =
	/** The server answered with an error status. */
	| "response"
	/** No answer arrived: offline, DNS, CORS, or a dropped connection. */
	| "network"
	| "timeout"
	/** A success status whose body could not be read. */
	| "malformed"
	/** A failure that did not come from the request itself. */
	| "unknown";

export interface ApiProblem {
	cause: ProblemCause;
	/** HTTP status, or null when no response arrived. */
	status: number | null;
	/** The server's classification; null when absent or unknown to this build. */
	kind: ErrorKind | null;
	/** Safe, standalone reason, shown after the screen's own context. */
	message: string;
	/** On a rate-limited response, the seconds to wait before retrying. */
	retryAfterSeconds?: number;
}

export const NETWORK_MESSAGE =
	"The server could not be reached. Check your connection and try again.";
export const TIMEOUT_MESSAGE =
	"The server took too long to respond. Try again.";
export const MALFORMED_MESSAGE =
	"The server sent a response this page could not read. Reload the page and try again.";
export const PERMISSION_MESSAGE = "You do not have permission to do that.";
export const UNKNOWN_MESSAGE = "No further detail is available.";

/** Compile-time exhaustive: a kind added to the API must be listed here. */
const ERROR_KINDS = {
	invalid_request: true,
	unauthorized: true,
	csrf_rejected: true,
	forbidden: true,
	not_found: true,
	clip_not_found: true,
	role_not_found: true,
	media_not_found: true,
	conflict: true,
	range_not_satisfiable: true,
	user_job_limit_reached: true,
	export_queue_full: true,
	execution_timed_out: true,
	media_temporarily_unavailable: true,
	archive_integrity_failure: true,
	media_inspection_failed: true,
	media_processing_failed: true,
	media_tools_unavailable: true,
	discord_rate_limited: true,
	discord_timeout: true,
	discord_unavailable: true,
	bot_unavailable: true,
	membership_unavailable: true,
	bot_not_in_voice: true,
	jam_cooldown: true,
	clip_playback_failed: true,
	worker_interrupted: true,
	source_changed: true,
	source_access_revoked: true,
	destination_changed: true,
	storage_budget_exceeded: true,
	waiting_for_media_work: true,
	unsafe_media_reference: true,
	internal_error: true,
} satisfies Record<ErrorKind, true>;

export function isErrorKind(value: unknown): value is ErrorKind {
	return typeof value === "string" && Object.hasOwn(ERROR_KINDS, value);
}

/** Prefixes older servers put in front of their 4xx messages. */
const LEGACY_PREFIX = /^(?:Bad Request|Conflict|Invalid path param): /;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function statusMessage(status: number): string {
	if (status === 401) return SESSION_EXPIRED_MESSAGE;
	if (status === 403) return PERMISSION_MESSAGE;
	if (status === 404) return "It was not found. It may have been deleted.";
	if (status === 429) return "Too many requests. Wait a moment and try again.";
	if (status >= 500)
		return `The server had a problem (HTTP ${status}). Try again later.`;
	return `The server rejected the request (HTTP ${status}).`;
}

/** Interpret an error body, falling back on its status when it says nothing. */
export function problemFromBody(status: number, body: unknown): ApiProblem {
	const problem = (message: string, kind: ErrorKind | null = null) => ({
		cause: "response" as const,
		status,
		kind,
		message,
	});
	if (isRecord(body) && typeof body.message === "string") {
		const message = body.message.trim();
		// A body with any `kind` follows the safe-message contract, including
		// kinds newer than this build.
		if (typeof body.kind === "string" && message) {
			const kind = isErrorKind(body.kind) ? body.kind : null;
			// The session message owns 401s so every screen words it the same.
			const contract = problem(
				status === 401 ? SESSION_EXPIRED_MESSAGE : message,
				kind,
			);
			const retryAfter = body.retry_after_seconds;
			return typeof retryAfter === "number" &&
				Number.isFinite(retryAfter) &&
				retryAfter >= 0
				? { ...contract, retryAfterSeconds: retryAfter }
				: contract;
		}
		// Older servers sent kind-less bodies. Their 4xx text was public; their
		// 5xx text was only a status phrase and says nothing useful.
		if (message && status >= 400 && status < 500 && status !== 401) {
			return problem(message.replace(LEGACY_PREFIX, ""));
		}
	}
	return problem(statusMessage(status));
}

/** Read a failed `fetch` response. Never throws. */
export async function problemFromResponse(
	response: Response,
): Promise<ApiProblem> {
	let body: unknown = null;
	try {
		const text = await response.text();
		body = text ? JSON.parse(text) : null;
	} catch {
		// An empty, HTML, or truncated body falls back on the status.
	}
	return problemFromBody(response.status, body);
}

function isFetchBaseQueryError(value: unknown): value is FetchBaseQueryError {
	return isRecord(value) && "status" in value;
}

/**
 * Interpret an RTK Query error (or a thrown value) from `.unwrap()`.
 * `unknown` is the reason given when nothing about the failure is known.
 */
export function problemFromQueryError(
	error: FetchBaseQueryError | SerializedError | undefined | null,
	unknown = UNKNOWN_MESSAGE,
): ApiProblem {
	if (!error || !isFetchBaseQueryError(error)) {
		return { cause: "unknown", status: null, kind: null, message: unknown };
	}
	if (typeof error.status === "number") {
		return problemFromBody(error.status, error.data);
	}
	switch (error.status) {
		case "FETCH_ERROR":
			return {
				cause: "network",
				status: null,
				kind: null,
				message: NETWORK_MESSAGE,
			};
		case "TIMEOUT_ERROR":
			return {
				cause: "timeout",
				status: null,
				kind: null,
				message: TIMEOUT_MESSAGE,
			};
		case "PARSING_ERROR":
			return error.originalStatus >= 400
				? problemFromBody(error.originalStatus, null)
				: {
						cause: "malformed",
						status: error.originalStatus,
						kind: null,
						message: MALFORMED_MESSAGE,
					};
		default:
			return { cause: "unknown", status: null, kind: null, message: unknown };
	}
}

/** Carries a normalized problem through code that throws. */
export class ApiRequestError extends Error {
	readonly problem: ApiProblem;

	constructor(problem: ApiProblem) {
		super(problem.message);
		this.name = "ApiRequestError";
		this.problem = problem;
	}
}

/** Interpret anything caught around a request. */
export function problemFromError(
	error: unknown,
	unknown = UNKNOWN_MESSAGE,
): ApiProblem {
	if (error instanceof ApiRequestError) return error.problem;
	if (isFetchBaseQueryError(error))
		return problemFromQueryError(error, unknown);
	// `fetch` rejects with a TypeError when no response arrives.
	if (error instanceof TypeError) {
		return {
			cause: "network",
			status: null,
			kind: null,
			message: NETWORK_MESSAGE,
		};
	}
	if (error instanceof DOMException && error.name === "TimeoutError") {
		return {
			cause: "timeout",
			status: null,
			kind: null,
			message: TIMEOUT_MESSAGE,
		};
	}
	return { cause: "unknown", status: null, kind: null, message: unknown };
}

/** Resolve a response, throwing an {@link ApiRequestError} unless it is OK. */
export async function ensureOk(response: Response): Promise<Response> {
	if (response.ok) return response;
	throw new ApiRequestError(await problemFromResponse(response));
}

/** Parse a successful JSON body, reporting an unreadable one as such. */
export async function readJson<T>(
	response: Response,
	isValid: (value: unknown) => value is T,
): Promise<T> {
	let value: unknown;
	try {
		value = await response.json();
	} catch {
		value = undefined;
	}
	if (isValid(value)) return value;
	throw new ApiRequestError({
		cause: "malformed",
		status: response.status,
		kind: null,
		message: MALFORMED_MESSAGE,
	});
}

/**
 * Whether the server definitely did not act on a request. Without a response,
 * or after an unclassified server fault, a mutation may have committed and
 * must be treated as uncertain.
 */
export function isDefiniteRejection(problem: ApiProblem): boolean {
	if (problem.cause !== "response" || problem.status === null) return false;
	if (problem.status < 500) return true;
	return problem.kind !== null && problem.kind !== "internal_error";
}

/** Whether repeating the same request later could reasonably succeed. */
export function isTransient(problem: ApiProblem): boolean {
	if (problem.cause === "network" || problem.cause === "timeout") return true;
	if (problem.status === 429) return true;
	if (problem.kind === "archive_integrity_failure") return false;
	if (problem.kind === "media_tools_unavailable") return false;
	return problem.status !== null && problem.status >= 500;
}

/**
 * Explain a failed action that requires the Manage Server permission.
 * Validation and conflict messages already say what to change; anything else
 * follows `context`, which says what failed.
 */
export function managerActionFailure(error: unknown, context: string): string {
	const problem = problemFromError(error);
	if (problem.kind === "forbidden" || problem.status === 403)
		return `${context} This requires the Manage Server permission.`;
	if (problem.kind === "invalid_request" || problem.kind === "conflict")
		return problem.message;
	return `${context} ${problem.message}`;
}
