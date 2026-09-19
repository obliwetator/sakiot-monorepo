import { API_ROUTES, apiUrl } from "../api/routes";
import { authedFetch } from "./authedFetch";

export interface MediaJobStatus {
	id: string;
	kind: string;
	status: "queued" | "running" | "ready" | "failed";
	stage: string;
	progress: number;
	result_url?: string | null;
	error?: string | null;
}

const POLL_INTERVAL_MS = 1_000;
const MAX_POLLS = 1_800;

function delay(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal?.aborted)
			return reject(new DOMException("Aborted", "AbortError"));
		const timer = window.setTimeout(resolve, ms);
		signal?.addEventListener(
			"abort",
			() => {
				window.clearTimeout(timer);
				reject(new DOMException("Aborted", "AbortError"));
			},
			{ once: true },
		);
	});
}

export async function waitForMediaJob(
	initial: MediaJobStatus,
	signal?: AbortSignal,
): Promise<MediaJobStatus> {
	let job = initial;
	for (let poll = 0; poll < MAX_POLLS; poll += 1) {
		if (job.status === "ready") return job;
		if (job.status === "failed") {
			throw new Error(job.error ?? "Media processing failed");
		}
		await delay(POLL_INTERVAL_MS, signal);
		const response = await authedFetch(
			apiUrl(API_ROUTES.mediaJob, { job_id: job.id }),
			{ signal },
		);
		if (!response.ok)
			throw new Error(`Media job status failed (${response.status})`);
		job = (await response.json()) as MediaJobStatus;
	}
	throw new Error("Media processing timed out");
}
