export const BASE_API_URL =
	(import.meta.env.VITE_API_URL as string | undefined) ||
	"https://dev.patrykstyla.com/api/";

export function getCsrfToken(): string | null {
	const match = document.cookie.match(/(?:^|;\s*)xsrf_token=([^;]*)/);
	return match ? match[1] : null;
}

export function isLoggedIn(): boolean {
	return /(?:^|;\s*)logged_in=1(?:;|$)/.test(document.cookie);
}

let refreshInFlight: Promise<boolean> | null = null;

export function ensureRefreshed(): Promise<boolean> {
	if (refreshInFlight) return refreshInFlight;
	refreshInFlight = (async () => {
		try {
			const headers = new Headers();
			const csrf = getCsrfToken();
			if (csrf) headers.set("X-CSRF-Token", csrf);
			const res = await fetch(`${BASE_API_URL}refresh`, {
				method: "POST",
				credentials: "include",
				headers,
			});
			return res.ok;
		} catch {
			return false;
		} finally {
			refreshInFlight = null;
		}
	})();
	return refreshInFlight;
}

function buildHeaders(init: RequestInit): Headers {
	const headers = new Headers(init.headers);
	const method = (init.method ?? "GET").toUpperCase();
	if (method !== "GET" && method !== "HEAD") {
		const csrf = getCsrfToken();
		if (csrf) headers.set("X-CSRF-Token", csrf);
	}
	return headers;
}

function resolveUrl(path: string): string {
	return /^https?:\/\//.test(path) ? path : BASE_API_URL + path;
}

export async function authedFetch(
	path: string,
	init: RequestInit = {},
): Promise<Response> {
	const url = resolveUrl(path);
	const headers = buildHeaders(init);
	const opts: RequestInit = { ...init, headers, credentials: "include" };

	let res = await fetch(url, opts);
	if (res.status !== 401) return res;

	const ok = await ensureRefreshed();
	if (ok) {
		const retryHeaders = buildHeaders(init);
		const retryOpts: RequestInit = {
			...init,
			headers: retryHeaders,
			credentials: "include",
		};
		res = await fetch(url, retryOpts);
	}
	return res;
}
