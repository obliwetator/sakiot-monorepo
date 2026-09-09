import { expect, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

test("a failed restore reports an error instead of failing silently", async ({
	page,
}) => {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
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
			await json({
				avatar: "",
				is_dev: false,
				user_id: "current-user",
				username: "Test Admin",
			});
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
		const voiceSettings = `/api/admin/guilds/${GUILD_ID}/voice-settings`;
		if (path === voiceSettings && request.method() === "GET") {
			await json({ pending_cap_seconds: 21_600, is_default: false });
			return;
		}
		if (path === voiceSettings && request.method() === "DELETE") {
			await json({ detail: "reset failed" }, 500);
			return;
		}
		await json(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});

	await page.goto(`/dashboard/${GUILD_ID}/admin/voice-settings`);
	await expect(
		page.getByRole("heading", { name: "Voice Settings", exact: true }),
	).toBeVisible();
	await page
		.getByRole("button", { name: "Restore six-hour default", exact: true })
		.click();
	await expect(
		page.getByText("Could not restore the default. Try again."),
	).toBeVisible();
});
