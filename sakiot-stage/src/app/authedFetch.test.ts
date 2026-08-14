import { afterEach, describe, expect, it, mock } from "bun:test";
import {
	authedFetch,
	BASE_API_URL,
	getCsrfToken,
	isLoggedIn,
	refreshForMediaRetry,
	SESSION_EXPIRED_MESSAGE,
	setCsrfToken,
} from "./authedFetch";

const originalDocument = globalThis.document;
const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;

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
		const fetchMock = mock(async () => new Response("ok", { status: 200 }));
		globalThis.fetch = fetchMock as typeof fetch;

		await authedFetch("clips", { method: "POST", body: "x" });

		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0];
		expect(url).toBe(`${BASE_API_URL}clips`);
		expect(init?.credentials).toBe("include");
		expect(new Headers(init?.headers).get("X-CSRF-Token")).toBe("csrf-123");
	});

	it("resolves API-root-relative media URLs without duplicating the API prefix", async () => {
		setCookie("logged_in=1");
		const fetchMock = mock(async () => new Response("ok", { status: 200 }));
		globalThis.fetch = fetchMock as typeof fetch;

		await authedFetch("/api/audio/waveform/recording");

		expect(fetchMock.mock.calls[0][0]).toBe(
			`${new URL(BASE_API_URL).origin}/api/audio/waveform/recording`,
		);
	});

	it("refreshes once and retries after a 401", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = mock(async (url: RequestInfo | URL) => {
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
		globalThis.fetch = fetchMock as typeof fetch;

		const res = await authedFetch("protected");

		expect(res.status).toBe(200);
		expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
			`${BASE_API_URL}protected`,
			`${BASE_API_URL}refresh`,
			`${BASE_API_URL}protected`,
		]);
		const [, refreshInit] = fetchMock.mock.calls[1];
		expect(refreshInit?.method).toBe("POST");
		expect(new Headers(refreshInit?.headers).get("X-CSRF-Token")).toBe(
			"csrf-123",
		);
	});
});

describe("refreshForMediaRetry", () => {
	it("reports a live session when the refresh succeeds", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = mock(
			async () => new Response("refreshed", { status: 200 }),
		);
		globalThis.fetch = fetchMock as typeof fetch;

		await expect(refreshForMediaRetry()).resolves.toBe(true);
	});

	it("reports an expired session when the refresh is rejected", async () => {
		setCookie("xsrf_token=csrf-123; logged_in=1");
		const fetchMock = mock(
			async () => new Response("expired", { status: 401 }),
		);
		globalThis.fetch = fetchMock as typeof fetch;

		await expect(refreshForMediaRetry()).resolves.toBe(false);
		expect(SESSION_EXPIRED_MESSAGE).toContain("session has expired");
	});
});
