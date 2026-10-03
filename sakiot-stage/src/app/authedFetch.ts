const configuredApiUrl = import.meta.env.VITE_API_URL as string | undefined;
if (!configuredApiUrl) {
	throw new Error(
		"VITE_API_URL is not set. Every build must provide the API origin (see .env.example); there is no fallback.",
	);
}
export const BASE_API_URL = configuredApiUrl;

const CSRF_STORAGE_KEY = "sakiot.csrf";
/** Fallback when storage is unavailable (private or restricted contexts). */
let csrfOverride: string | null = null;
/**
 * Bumped whenever the token changes (login, logout, another tab logging in).
 * A response to a request that started under an earlier account must not
 * write its token back.
 */
let accountEpoch = 0;

export function currentAccountEpoch(): number {
	return accountEpoch;
}

/** The stored token, `null` when none is stored, `undefined` without storage. */
function readStoredCsrf(): string | null | undefined {
	try {
		return localStorage.getItem(CSRF_STORAGE_KEY);
	} catch {
		return undefined;
	}
}

export function setCsrfToken(value: string | null): void {
	const stored = readStoredCsrf();
	const previous = stored === undefined ? csrfOverride : stored;
	if (value !== previous) accountEpoch += 1;
	csrfOverride = value;
	try {
		if (value) localStorage.setItem(CSRF_STORAGE_KEY, value);
		else localStorage.removeItem(CSRF_STORAGE_KEY);
	} catch {
		// Storage can be unavailable in private or restricted browser contexts.
	}
}

/**
 * The token is read from shared storage at the moment it is used, so a login
 * or logout in another tab of this origin applies here at once. The token is
 * fixed for a whole login (the server keeps it across refreshes), so tabs
 * never invalidate each other's copy.
 */
export function getCsrfToken(): string | null {
	const stored = readStoredCsrf();
	if (stored) return stored;
	if (stored === undefined && csrfOverride) return csrfOverride;
	if (typeof document === "undefined") return null;
	const matches = [
		...document.cookie.matchAll(
			/(?:^|;\s*)(?:__Host-sakiot-xsrf_token|xsrf_token)=([^;]*)/g,
		),
	];
	return matches.at(-1)?.[1] ?? null;
}

/** Login and refresh responses carry the session's token: always take it. */
export function captureCsrfToken(response: Response): void {
	const csrf = response.headers.get("X-CSRF-Token");
	if (csrf) setCsrfToken(csrf);
}

/**
 * Any other authenticated response echoes the token its request was sent
 * with. Use it only to recover a token this tab lost (cleared storage on a
 * cross-origin deployment), and never from a request that started before the
 * account changed: a late response must not restore a stale token.
 */
export function healCsrfToken(
	response: Response,
	epochAtRequest: number,
): void {
	const csrf = response.headers.get("X-CSRF-Token");
	if (!csrf || epochAtRequest !== accountEpoch || getCsrfToken() !== null) {
		return;
	}
	setCsrfToken(csrf);
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

/**
 * - `refreshed`: a new access token is in the cookies (this tab's refresh or
 *   another tab's, which shares them).
 * - `session-ended`: the refresh token is gone (401): clear private state.
 * - `unavailable`: a network error, 5xx, or `csrf_rejected`. Temporary: keep
 *   drafts, retry later, never log out for it.
 */
export type RefreshOutcome = "refreshed" | "session-ended" | "unavailable";

const REFRESHED_AT_KEY = "sakiot.refreshedAt";
let refreshInFlight: Promise<RefreshOutcome> | null = null;

function readRefreshedAt(): number {
	try {
		return Number(localStorage.getItem(REFRESHED_AT_KEY) ?? 0) || 0;
	} catch {
		return 0;
	}
}

function writeRefreshedAt(at: number): void {
	try {
		localStorage.setItem(REFRESHED_AT_KEY, String(at));
	} catch {
		// Without storage each tab simply refreshes for itself.
	}
}

/**
 * Serializes refreshes across this origin's tabs. Correctness does not depend
 * on it: the staging and debug pages share the API's cookies but not locks or
 * storage, and concurrent refreshes are harmless because the CSRF token is
 * fixed for the login. It only saves redundant calls.
 */
async function withRefreshLock<T>(work: () => Promise<T>): Promise<T> {
	const locks = typeof navigator === "undefined" ? undefined : navigator.locks;
	if (!locks?.request) return work();
	return locks.request("sakiot-refresh", work);
}

async function postRefresh(): Promise<RefreshOutcome> {
	try {
		const headers = new Headers();
		const csrf = getCsrfToken();
		if (csrf) headers.set("X-CSRF-Token", csrf);
		const res = await fetch(`${BASE_API_URL}refresh`, {
			method: "POST",
			credentials: "include",
			headers,
		});
		if (res.ok) {
			captureCsrfToken(res);
			writeRefreshedAt(Date.now());
			return "refreshed";
		}
		return res.status === 401 ? "session-ended" : "unavailable";
	} catch {
		return "unavailable";
	}
}

/**
 * Renews the access token, unless another tab already did after
 * `skipIfRefreshedAfter` (unix ms): the cookies are shared, so its new token
 * is already ours.
 */
export function refreshSession(
	skipIfRefreshedAfter = Date.now() - 10_000,
): Promise<RefreshOutcome> {
	if (refreshInFlight) return refreshInFlight;
	refreshInFlight = withRefreshLock(async () => {
		if (readRefreshedAt() > skipIfRefreshedAfter) return "refreshed" as const;
		return postRefresh();
	}).finally(() => {
		refreshInFlight = null;
	});
	return refreshInFlight;
}

export const SESSION_EXPIRED_MESSAGE =
	"Your session has expired. Please log in again.";

/**
 * Media elements cannot read the HTTP status of a failed load, so a stale
 * access token surfaces as a plain playback error. Before surfacing one,
 * refresh the session and report whether a retry is worth attempting: true
 * when a new access token was issued (reload the media), false when the
 * refresh token has expired too (the session is over — show
 * SESSION_EXPIRED_MESSAGE instead of a misleading playback error).
 */
export async function refreshForMediaRetry(): Promise<boolean> {
	return ensureRefreshed();
}

export async function ensureRefreshed(): Promise<boolean> {
	return (await refreshSession()) === "refreshed";
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
	if (/^https?:\/\//.test(path)) return path;

	const base = new URL(BASE_API_URL);
	// API responses use root-relative paths (for example, /api/audio/...).
	// Resolve those against the origin so the /api prefix is not duplicated.
	if (path.startsWith("/")) return new URL(path, base.origin).toString();
	return new URL(path.replace(/^\/+/, ""), base).toString();
}

export async function authedFetch(
	path: string,
	init: RequestInit = {},
): Promise<Response> {
	const url = resolveUrl(path);
	const headers = buildHeaders(init);
	const opts: RequestInit = { ...init, headers, credentials: "include" };

	const epoch = accountEpoch;
	let res = await fetch(url, opts);
	healCsrfToken(res, epoch);
	if (res.status !== 401) return res;

	const ok = await ensureRefreshed();
	if (ok) {
		const retryHeaders = buildHeaders(init);
		const retryOpts: RequestInit = {
			...init,
			headers: retryHeaders,
			credentials: "include",
		};
		const retryEpoch = accountEpoch;
		res = await fetch(url, retryOpts);
		healCsrfToken(res, retryEpoch);
	}
	return res;
}
