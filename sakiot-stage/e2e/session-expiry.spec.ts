import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

/**
 * Serves a valid session once, then expires it: data calls 401, the refresh
 * endpoint 401s too, and the auth probe fails on its next fetch.
 */
async function mockExpiredSession(page: Page) {
	let authCalls = 0;
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname;

		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		const json = async (body: unknown, status = 200) => {
			await route.fulfill({
				status,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};

		if (path === "/api/users/current") {
			authCalls += 1;
			if (authCalls === 1) {
				await json({
					avatar: "",
					is_dev: false,
					user_id: "current-user",
					username: "Test Admin",
				});
			} else {
				await json({ detail: "expired" }, 401);
			}
			return;
		}
		if (path === "/api/users/current/guilds") {
			await json([
				{
					id: GUILD_ID,
					name: "Test Guild",
					owner: true,
					permissions: "8",
				},
			]);
			return;
		}
		if (path === "/api/refresh") {
			await json({ detail: "expired" }, 401);
			return;
		}
		// Every data request is now unauthorized.
		await json({ detail: "expired" }, 401);
	});
	return () => authCalls;
}

test("a failed token refresh drops the shell back to logged out", async ({
	page,
}, testInfo) => {
	const authCalls = await mockExpiredSession(page);
	await page.goto(`/dashboard/${GUILD_ID}/clips`);

	// The clips request 401s, the refresh 401s, and the invalidated auth probe
	// must then flip the shell instead of leaving a logged-in UI whose every
	// action fails.
	await expect(page.getByText("You are not logged in")).toBeVisible({
		timeout: 15_000,
	});
	if (testInfo.project.name === "mobile-chromium") {
		await page.getByRole("button", { name: "open navigation" }).click();
	}
	await expect(
		page.getByRole("button", { name: "Login", exact: true }),
	).toBeVisible();
	// The invalidated probe refetches once; it must not re-invalidate itself
	// into a request storm. Sample twice: an ongoing loop could be below the
	// cap at the instant the login screen appears.
	expect(authCalls()).toBeGreaterThan(1);
	expect(authCalls()).toBeLessThan(10);
	const settled = authCalls();
	await page.waitForTimeout(1_500);
	expect(authCalls()).toBe(settled);
});
