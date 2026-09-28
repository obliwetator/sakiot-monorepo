import { describe, expect, it } from "bun:test";
import {
	ApiRequestError,
	isDefiniteRejection,
	isTransient,
	MALFORMED_MESSAGE,
	managerActionFailure,
	NETWORK_MESSAGE,
	problemFromError,
	problemFromQueryError,
	problemFromResponse,
	readJson,
	TIMEOUT_MESSAGE,
	UNKNOWN_MESSAGE,
} from "./apiError";
import { SESSION_EXPIRED_MESSAGE } from "./authedFetch";

function response(status: number, body: string, type = "application/json") {
	return new Response(body, { status, headers: { "Content-Type": type } });
}

describe("problemFromResponse", () => {
	it("prefers the safe API contract", async () => {
		const problem = await problemFromResponse(
			response(
				503,
				JSON.stringify({
					code: 503,
					kind: "user_job_limit_reached",
					message: "You already have the maximum number of active media jobs.",
				}),
			),
		);
		expect(problem).toEqual({
			cause: "response",
			status: 503,
			kind: "user_job_limit_reached",
			message: "You already have the maximum number of active media jobs.",
		});
		expect(isDefiniteRejection(problem)).toBe(true);
	});

	it("keeps the message of a kind this build does not know", async () => {
		const problem = await problemFromResponse(
			response(
				409,
				JSON.stringify({
					code: 409,
					kind: "kind_from_a_newer_server",
					message: "Explained by the server.",
				}),
			),
		);
		expect(problem.kind).toBeNull();
		expect(problem.message).toBe("Explained by the server.");
	});

	it("accepts older 4xx bodies without a kind and drops their prefix", async () => {
		const problem = await problemFromResponse(
			response(
				400,
				JSON.stringify({
					code: 400,
					message:
						"Bad Request: Clip duration must be between 1 and 20 seconds",
				}),
			),
		);
		expect(problem.message).toBe(
			"Clip duration must be between 1 and 20 seconds",
		);
		expect(problem.kind).toBeNull();
	});

	it("ignores older 5xx text and names the failure instead", async () => {
		const problem = await problemFromResponse(
			response(
				500,
				JSON.stringify({ code: 500, message: "Internal Server Error" }),
			),
		);
		expect(problem.message).toBe(
			"The server had a problem (HTTP 500). Try again later.",
		);
		expect(isDefiniteRejection(problem)).toBe(false);
		expect(isTransient(problem)).toBe(true);
	});

	it("falls back on the status for empty, HTML, and truncated bodies", async () => {
		for (const body of [
			"",
			"<html><body>502 Bad Gateway</body></html>",
			'{"code": 50',
		]) {
			const problem = await problemFromResponse(
				response(502, body, "text/html"),
			);
			expect(problem.message).toBe(
				"The server had a problem (HTTP 502). Try again later.",
			);
			expect(problem.message).not.toContain("html");
		}
	});

	it("words session expiry and permission failures consistently", async () => {
		const expired = await problemFromResponse(
			response(401, JSON.stringify({ error: "expired_or_invalid_token" })),
		);
		expect(expired.message).toBe(SESSION_EXPIRED_MESSAGE);
		const forbidden = await problemFromResponse(response(403, ""));
		expect(forbidden.message).toBe("You do not have permission to do that.");
	});
});

describe("problemFromQueryError", () => {
	it("reads the contract from an RTK Query error", () => {
		const problem = problemFromQueryError({
			status: 503,
			data: {
				code: 503,
				kind: "export_queue_full",
				message: "The export queue is full.",
			},
		});
		expect(problem.kind).toBe("export_queue_full");
		expect(problem.message).toBe("The export queue is full.");
	});

	it("separates network failures and timeouts from server answers", () => {
		const network = problemFromQueryError({
			status: "FETCH_ERROR",
			error: "TypeError: Failed to fetch",
		});
		expect(network).toEqual({
			cause: "network",
			status: null,
			kind: null,
			message: NETWORK_MESSAGE,
		});
		expect(isDefiniteRejection(network)).toBe(false);
		expect(
			problemFromQueryError({ status: "TIMEOUT_ERROR", error: "AbortError" })
				.message,
		).toBe(TIMEOUT_MESSAGE);
	});

	it("reports a malformed success body without echoing parser text", () => {
		const problem = problemFromQueryError({
			status: "PARSING_ERROR",
			originalStatus: 200,
			data: "<html>",
			error: "SyntaxError: Unexpected token <",
		});
		expect(problem.cause).toBe("malformed");
		expect(problem.message).toBe(MALFORMED_MESSAGE);
		const failed = problemFromQueryError({
			status: "PARSING_ERROR",
			originalStatus: 504,
			data: "<html>",
			error: "SyntaxError",
		});
		expect(failed.message).toContain("HTTP 504");
		expect(failed.message).not.toContain("SyntaxError");
	});

	it("never displays serialized exceptions or custom error text", () => {
		expect(
			problemFromQueryError({
				name: "Error",
				message: "Cannot read properties of undefined",
			}).message,
		).toBe(UNKNOWN_MESSAGE);
		expect(
			problemFromQueryError({
				status: "CUSTOM_ERROR",
				error: "stack trace here",
			}).message,
		).toBe(UNKNOWN_MESSAGE);
		expect(problemFromQueryError(undefined).message).toBe(UNKNOWN_MESSAGE);
	});
});

describe("problemFromError", () => {
	it("recognizes fetch network failures and thrown problems", () => {
		expect(problemFromError(new TypeError("Failed to fetch")).cause).toBe(
			"network",
		);
		const problem = {
			cause: "response" as const,
			status: 404,
			kind: "media_not_found" as const,
			message: "Gone.",
		};
		expect(problemFromError(new ApiRequestError(problem))).toBe(problem);
		expect(problemFromError(new Error("internal detail"))).toEqual({
			cause: "unknown",
			status: null,
			kind: null,
			message: UNKNOWN_MESSAGE,
		});
	});
});

describe("readJson", () => {
	const isJob = (value: unknown): value is { id: string } =>
		typeof value === "object" &&
		value !== null &&
		typeof (value as { id?: unknown }).id === "string";

	it("returns valid bodies and rejects malformed or unexpected ones", async () => {
		expect(await readJson(response(200, '{"id":"a"}'), isJob)).toEqual({
			id: "a",
		});
		for (const body of ["", "not json", '{"other":1}']) {
			const error = await readJson(response(200, body), isJob).catch(
				(caught: unknown) => caught,
			);
			expect(error).toBeInstanceOf(ApiRequestError);
			expect((error as ApiRequestError).problem.cause).toBe("malformed");
		}
	});
});

describe("managerActionFailure", () => {
	const context = "The recording policy was not saved.";
	it("names the permission a forbidden action needs", () => {
		expect(
			managerActionFailure(
				{
					status: 403,
					data: { code: 403, kind: "forbidden", message: "No." },
				},
				context,
			),
		).toBe(`${context} This requires the Manage Server permission.`);
	});

	it("shows validation text alone and prefixes everything else once", () => {
		expect(
			managerActionFailure(
				{
					status: 400,
					data: {
						code: 400,
						kind: "invalid_request",
						message: "Retention must be between 1 and 3650 days.",
					},
				},
				context,
			),
		).toBe("Retention must be between 1 and 3650 days.");
		expect(managerActionFailure({ status: 500, data: null }, context)).toBe(
			`${context} The server had a problem (HTTP 500). Try again later.`,
		);
		expect(managerActionFailure(new Error("secret"), context)).toBe(
			`${context} ${UNKNOWN_MESSAGE}`,
		);
	});
});
