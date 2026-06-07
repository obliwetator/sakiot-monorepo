import { afterEach, describe, expect, it, mock } from "bun:test";
import {
	authedFetch,
	BASE_API_URL,
	getCsrfToken,
	isLoggedIn,
} from "./authedFetch";

const originalDocument = globalThis.document;
const originalFetch = globalThis.fetch;

function setCookie(cookie: string) {
	Object.defineProperty(globalThis, "document", {
		configurable: true,
		value: { cookie },
	});
}

afterEach(() => {
	Object.defineProperty(globalThis, "document", {
		configurable: true,
		value: originalDocument,
	});
	globalThis.fetch = originalFetch;
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
