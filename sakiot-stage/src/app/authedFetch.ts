const configuredApiUrl = import.meta.env.VITE_API_URL as string | undefined;
if (!configuredApiUrl) {
	throw new Error(
		"VITE_API_URL is not set. Every build must provide the API origin (see .env.example); there is no fallback.",
	);
}
export const BASE_API_URL = configuredApiUrl;

const CSRF_STORAGE_KEY = "sakiot.csrf";
let csrfOverride: string | null = null;

export function setCsrfToken(value: string | null): void {
	csrfOverride = value;
	try {
		if (value) localStorage.setItem(CSRF_STORAGE_KEY, value);
		else localStorage.removeItem(CSRF_STORAGE_KEY);
	} catch {
		// Storage can be unavailable in private or restricted browser contexts.
	}
}

export function getCsrfToken(): string | null {
	if (csrfOverride) return csrfOverride;
	try {
		const stored = localStorage.getItem(CSRF_STORAGE_KEY);
		if (stored) return stored;
	} catch {
		// Fall back to a same-origin cookie below.
	}
	const matches = [
		...document.cookie.matchAll(
			/(?:^|;\s*)(?:__Host-sakiot-xsrf_token|xsrf_token)=([^;]*)/g,
		),
	];
	return matches.length > 0 ? matches[matches.length - 1][1] : null;
}

export function captureCsrfToken(response: Response): void {
	const csrf = response.headers.get("X-CSRF-Token");
	if (csrf) setCsrfToken(csrf);
}

export function isLoggedIn(): boolean {
	// Staging deploys the bundle on staging.patrykstyla.com while
	// VITE_API_URL points at debug.patrykstyla.com/api/, so the host-only
	// auth cookies are scoped to the API origin and never appear in this
	// page's document.cookie. In that topology login state can only be
	// decided by probing the API; report "unknown" as logged in so the
	// skip-gated queries actually run (they fall back to the login screen
	// on 401).
	if (
		typeof window !== "undefined" &&
		new URL(BASE_API_URL, window.location.origin).origin !==
			window.location.origin
	) {
		return true;
	}
	return /(?:^|;\s*)(?:__Host-sakiot-logged_in|logged_in)=1(?:;|$)/.test(
		document.cookie,
	);
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
			captureCsrfToken(res);
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
	captureCsrfToken(res);
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
		captureCsrfToken(res);
	}
	return res;
}
