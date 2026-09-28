import { afterEach, describe, expect, it } from "bun:test";
import { NETWORK_MESSAGE } from "./apiError";
import {
	MediaJobFailedError,
	type MediaJobStatus,
	MediaJobUnreachableError,
	waitForMediaJob,
} from "./mediaJobs";

const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});

const queued: MediaJobStatus = {
	id: "job-1",
	kind: "session_download",
	status: "queued",
	stage: "queued",
	progress: 0,
};
const noSleep = async () => {};

/** Serve one scripted outcome per poll; `Error` values reject like fetch does. */
function script(outcomes: (Response | Error)[]) {
	const calls: string[] = [];
	globalThis.fetch = (async (input: RequestInfo | URL) => {
		calls.push(String(input));
		const next = outcomes.shift();
		if (!next) throw new Error("unexpected poll");
		if (next instanceof Error) throw next;
		return next;
	}) as typeof fetch;
	return calls;
}

const json = (status: number, body: unknown) =>
	new Response(JSON.stringify(body), {
		status,
		headers: { "Content-Type": "application/json" },
	});

describe("waitForMediaJob", () => {
	it("rides out transient poll failures without declaring the job failed", async () => {
		script([
			new TypeError("Failed to fetch"),
			new Response("", { status: 502 }),
			json(200, { ...queued, status: "running" }),
			json(200, { ...queued, status: "ready", result_url: "/r" }),
		]);
		const job = await waitForMediaJob(queued, { sleep: noSleep });
		expect(job.status).toBe("ready");
	});

	it("reports lost contact separately from a confirmed failure", async () => {
		script(Array.from({ length: 5 }, () => new TypeError("Failed to fetch")));
		const error = await waitForMediaJob(queued, { sleep: noSleep }).catch(
			(caught: unknown) => caught,
		);
		expect(error).toBeInstanceOf(MediaJobUnreachableError);
		expect(error).not.toBeInstanceOf(MediaJobFailedError);
		expect((error as MediaJobUnreachableError).problem.message).toBe(
			NETWORK_MESSAGE,
		);
	});

	it("stops at once when the job is no longer visible", async () => {
		script([
			json(404, { code: 404, kind: "not_found", message: "Not found." }),
		]);
		const error = await waitForMediaJob(queued, { sleep: noSleep }).catch(
			(caught: unknown) => caught,
		);
		expect(error).toBeInstanceOf(MediaJobUnreachableError);
		expect((error as MediaJobUnreachableError).problem.kind).toBe("not_found");
	});

	it("surfaces the server's safe failure message", async () => {
		script([
			json(200, {
				...queued,
				status: "failed",
				error: "The server could not process this media.",
				error_kind: "media_processing_failed",
			}),
		]);
		const error = await waitForMediaJob(queued, { sleep: noSleep }).catch(
			(caught: unknown) => caught,
		);
		expect(error).toBeInstanceOf(MediaJobFailedError);
		expect((error as MediaJobFailedError).message).toBe(
			"The server could not process this media.",
		);
	});

	it("treats an unreadable status body as lost contact, not failure", async () => {
		script(Array.from({ length: 5 }, () => json(200, { unexpected: true })));
		const error = await waitForMediaJob(queued, { sleep: noSleep }).catch(
			(caught: unknown) => caught,
		);
		expect(error).toBeInstanceOf(MediaJobUnreachableError);
	});
});
