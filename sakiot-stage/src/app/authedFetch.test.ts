import { afterEach, describe, expect, it, mock } from "bun:test";
import {
	authedFetch,
	BASE_API_URL,
	currentAccountEpoch,
	getCsrfToken,
	healCsrfToken,
	isLoggedIn,
	refreshForMediaRetry,
	refreshSession,
	SESSION_EXPIRED_MESSAGE,
	setCsrfToken,
} from "./authedFetch";

const originalDocument = globalThis.document;
const originalStorage = globalThis.localStorage;

/** A shared-storage stand-in, as another tab of this origin would see it. */
function installStorage(): Map<string, string> {
	const items = new Map<string, string>();
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			getItem: (key: string) => items.get(key) ?? null,
			setItem: (key: string, value: string) => void items.set(key, value),
			removeItem: (key: string) => void items.delete(key),
		},
	});
	return items;
}
const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;

type FetchInput = RequestInfo | URL;
/** Bun's `Mock` is not structurally a `typeof fetch` (no `preconnect`), and an
 * untyped `mock()` records a zero-length call tuple. Both are fixed by giving
 * the stub the fetch signature and casting through `unknown` at the seam. */
function fetchStub(
	handler: (input: FetchInput, init?: RequestInit) => Promise<Response>,
) {
	return mock(handler);
}

function installFetch(stub: ReturnType<typeof fetchStub>) {
	globalThis.fetch = stub as unknown as typeof fetch;
}

function setCookie(cookie: string) {
	Object.defineProperty(globalThis, "document", {
		configurable: true,
		value: { cookie },
	});
}

function setPageOrigin(origin: string) {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { location: { origin } },
	});
}

afterEach(() => {
	Object.defineProperty(globalThis, "document", {
		configurable: true,
		value: originalDocument,
	});
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: originalWindow,
	});
	globalThis.fetch = originalFetch;
	setCsrfToken(null);
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: originalStorage,
	});
	mock.restore();
});

describe("auth cookie helpers", () => {
	it("reads csrf and logged-in cookies", () => {
		setCookie("theme=dark; xsrf_token=csrf-123; logged_in=1");

		expect(getCsrfToken()).toBe("csrf-123");
		expect(isLoggedIn()).toBe(true);
	});

	it("returns falsey values when auth cookies are missing", () => {
		setCookie("theme=dark");

		expect(getCsrfToken()).toBeNull();
		expect(isLoggedIn()).toBe(false);
	});

	it("assumes logged in when the API is cross-origin and cookies are invisible", () => {
		setCookie("theme=dark");
		const apiOrigin = new URL(BASE_API_URL).origin;
		setPageOrigin(
			apiOrigin === "http://localhost:8081"
				? "http://localhost:8082"
				: "http://localhost:8081",
		);

		expect(isLoggedIn()).toBe(true);
	});

	it("consults cookies when the API is same-origin", () => {
		setPageOrigin(new URL(BASE_API_URL).origin);
		setCookie("theme=dark");

		expect(isLoggedIn()).toBe(false);

		setCookie("theme=dark; logged_in=1");
		expect(isLoggedIn()).toBe(true);
	});

	it("uses an explicitly received csrf token when the API cookie is not readable", () => {
		setCookie("theme=dark");
		setCsrfToken("csrf-from-api");

		expect(getCsrfToken()).toBe("csrf-from-api");
	});
});

describe("authedFetch", () => {
	it("adds credentials and csrf header for mutating relative requests", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = fetchStub(
			async () => new Response("ok", { status: 200 }),
		);
		installFetch(fetchMock);

		await authedFetch("clips", { method: "POST", body: "x" });

		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0] ?? [];
		expect(url).toBe(`${BASE_API_URL}clips`);
		expect(init?.credentials).toBe("include");
		expect(new Headers(init?.headers).get("X-CSRF-Token")).toBe("csrf-123");
	});

	it("resolves API-root-relative media URLs without duplicating the API prefix", async () => {
		setCookie("logged_in=1");
		const fetchMock = fetchStub(
			async () => new Response("ok", { status: 200 }),
		);
		installFetch(fetchMock);

		await authedFetch("/api/audio/waveform/recording");

		expect(fetchMock.mock.calls[0]?.[0]).toBe(
			`${new URL(BASE_API_URL).origin}/api/audio/waveform/recording`,
		);
	});

	it("refreshes once and retries after a 401", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = fetchStub(async (url: FetchInput) => {
			if (String(url).endsWith("protected")) {
				const count = fetchMock.mock.calls.filter(([callUrl]) =>
					String(callUrl).endsWith("protected"),
				).length;
				return new Response(count === 1 ? "unauthorized" : "ok", {
					status: count === 1 ? 401 : 200,
				});
			}
			if (String(url).endsWith("refresh")) {
				return new Response("refreshed", { status: 200 });
			}
			return new Response("unexpected", { status: 500 });
		});
		installFetch(fetchMock);

		const res = await authedFetch("protected");

		expect(res.status).toBe(200);
		expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
			`${BASE_API_URL}protected`,
			`${BASE_API_URL}refresh`,
			`${BASE_API_URL}protected`,
		]);
		const [, refreshInit] = fetchMock.mock.calls[1] ?? [];
		expect(refreshInit?.method).toBe("POST");
		expect(new Headers(refreshInit?.headers).get("X-CSRF-Token")).toBe(
			"csrf-123",
		);
	});
});

describe("refreshForMediaRetry", () => {
	it("reports a live session when the refresh succeeds", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = fetchStub(
			async () => new Response("refreshed", { status: 200 }),
		);
		installFetch(fetchMock);

		await expect(refreshForMediaRetry()).resolves.toBe(true);
	});

	it("reports an expired session when the refresh is rejected", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = fetchStub(
			async () => new Response("expired", { status: 401 }),
		);
		installFetch(fetchMock);

		await expect(refreshForMediaRetry()).resolves.toBe(false);
		expect(SESSION_EXPIRED_MESSAGE).toContain("session has expired");
	});
});

describe("session-stable CSRF", () => {
	it("reads the token from shared storage when it is used", () => {
		setCookie("");
		const storage = installStorage();
		setCsrfToken("tab-a-login");
		// Another tab of this origin logs in as someone else.
		storage.set("sakiot.csrf", "tab-b-login");
		expect(getCsrfToken()).toBe("tab-b-login");
	});

	it("never lets a late response restore a previous account's token", () => {
		setCookie("");
		installStorage();
		setCsrfToken("old-account");
		const before = currentAccountEpoch();
		setCsrfToken(null); // logged out while a request was in flight
		const late = new Response("ok", {
			headers: { "X-CSRF-Token": "old-account" },
		});
		healCsrfToken(late, before);
		expect(getCsrfToken()).toBeNull();

		// A response to a request made under the current account may recover
		// a token this tab lost.
		healCsrfToken(late, currentAccountEpoch());
		expect(getCsrfToken()).toBe("old-account");
	});

	it("does not overwrite a stored token from ordinary responses", () => {
		setCookie("");
		installStorage();
		setCsrfToken("current");
		healCsrfToken(
			new Response("ok", { headers: { "X-CSRF-Token": "other" } }),
			currentAccountEpoch(),
		);
		expect(getCsrfToken()).toBe("current");
	});
});

describe("refreshSession", () => {
	it("tells an ended session apart from a temporary failure", async () => {
		setCookie("xsrf_token=csrf-123");
		installStorage();
		for (const [status, outcome] of [
			[200, "refreshed"],
			[401, "session-ended"],
			[403, "unavailable"],
			[503, "unavailable"],
		] as const) {
			installFetch(fetchStub(async () => new Response("", { status })));
			await expect(refreshSession(0)).resolves.toBe(outcome);
			localStorage.removeItem("sakiot.refreshedAt");
		}
		installFetch(
			fetchStub(async () => {
				throw new TypeError("network down");
			}),
		);
		await expect(refreshSession(0)).resolves.toBe("unavailable");
	});

	it("skips the call when another tab already refreshed the shared cookies", async () => {
		setCookie("xsrf_token=csrf-123");
		const storage = installStorage();
		const fetchMock = fetchStub(async () => new Response("", { status: 200 }));
		installFetch(fetchMock);

		storage.set("sakiot.refreshedAt", String(Date.now()));
		await expect(refreshSession(Date.now() - 1_000)).resolves.toBe("refreshed");
		expect(fetchMock).toHaveBeenCalledTimes(0);

		await expect(refreshSession(Date.now() + 1_000)).resolves.toBe("refreshed");
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});
});
