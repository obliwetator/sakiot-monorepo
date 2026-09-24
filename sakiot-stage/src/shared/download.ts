import { authedFetch } from "../app/authedFetch";

/**
 * Hand a blob to the browser as a download.
 *
 * The anchor is attached to the document before clicking because a detached
 * anchor is ignored by some browsers, and the object URL is revoked on a timer
 * rather than synchronously: revoking immediately after `click()` can abort the
 * download before it starts.
 */
export function saveBlob(blob: Blob, fileName: string): void {
	const url = URL.createObjectURL(blob);
	try {
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = fileName;
		document.body.appendChild(anchor);
		anchor.click();
		anchor.remove();
	} catch {
		window.open(url, "_blank");
	} finally {
		window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
	}
}

export interface DownloadOptions {
	/** Sentence-cased prefix for the failure messages, e.g. "Clip download". */
	label: string;
	onError: (message: string) => void;
	/** Overrides the message used when the request rejects outright. */
	rejectionMessage?: string;
}

/**
 * Fetch an authed URL and save the body as a file, reporting both a failed
 * response and a dropped connection through `onError`. A rejection has no
 * status to report, so it gets its own message.
 */
export async function downloadAsFile(
	url: string,
	fileName: string,
	options: DownloadOptions,
): Promise<void> {
	try {
		const response = await authedFetch(url);
		if (!response.ok) {
			options.onError(`${options.label} failed (${response.status}).`);
			return;
		}
		saveBlob(await response.blob(), fileName);
	} catch {
		options.onError(options.rejectionMessage ?? `${options.label} failed.`);
	}
}
