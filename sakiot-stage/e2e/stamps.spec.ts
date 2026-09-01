import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

async function mockApi(page: Page) {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname.replace(API_PREFIX, "");

		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}

		const fulfillJson = async (body: unknown, status = 200) => {
			await route.fulfill({
				status,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};

		if (path === "/users/current") {
			await fulfillJson({
				avatar: "",
				is_dev: false,
				user_id: "current-user",
				username: "Test Admin",
			});
			return;
		}

		if (path === "/users/current/guilds") {
			await fulfillJson([
				{
					id: GUILD_ID,
					name: "Test Guild",
					owner: true,
					permissions: "8",
				},
			]);
			return;
		}

		if (path === `/stamps/${GUILD_ID}`) {
			await fulfillJson([
				{
					audio_file_id: 42,
					channel_id: "channel-123",
					channel_name: "General",
					created_at: "2026-08-11T12:30:00.000Z",
					guild_id: GUILD_ID,
					id: 9001,
					note: "Important moment",
					offset_ms: 250,
					recording_session_id: "session-123",
					segment_index: 1,
					session_fragment_count: 3,
					session_started_at_ms: 1_786_460_390_000,
					stamp_ts: 1_786_460_400_000,
					stamper_name: "Test Stamper",
					stamper_user_id: "10002",
					start_ts: 1_786_460_395_000,
					target_name: "Test Target",
					target_user_id: "10001",
				},
			]);
			return;
		}

		await fulfillJson(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
}

test("stamps headers and values share table columns", async ({ page }) => {
	await mockApi(page);
	await page.goto(`/stamps/${GUILD_ID}`);
	await expect(page.getByRole("heading", { name: /Stamps/ })).toBeVisible();

	const table = page.getByRole("table");
	const headers = table.locator("thead th");
	const cells = table.locator("tbody tr").first().locator("td");

	await expect(table.locator("thead")).toHaveCount(1);
	await expect(table.locator("thead tr")).toHaveCount(1);
	await expect(headers).toHaveCount(11);
	await expect(cells).toHaveCount(11);

	for (let column = 0; column < 11; column += 1) {
		const [headerBox, cellBox] = await Promise.all([
			headers.nth(column).boundingBox(),
			cells.nth(column).boundingBox(),
		]);
		expect(headerBox).not.toBeNull();
		expect(cellBox).not.toBeNull();
		if (headerBox && cellBox) {
			expect(Math.abs(headerBox.x - cellBox.x)).toBeLessThanOrEqual(1);
			expect(Math.abs(headerBox.width - cellBox.width)).toBeLessThanOrEqual(1);
		}
	}
});
