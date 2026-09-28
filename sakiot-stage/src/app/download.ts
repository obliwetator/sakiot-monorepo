import { ensureOk } from "./apiError";
import { authedFetch } from "./authedFetch";

/** Hand a blob to the browser as a file download. */
export function saveBlob(blob: Blob, fileName: string) {
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

/**
 * Fetch an authenticated file and save it. Rejects with an `ApiRequestError`
 * (or a network error) that `problemFromError` can explain.
 */
export async function downloadFile(
	path: string,
	fileName: string,
): Promise<void> {
	const response = await ensureOk(await authedFetch(path));
	saveBlob(await response.blob(), fileName);
}
