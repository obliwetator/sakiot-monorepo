import { expect, fulfillSharedRoutes, test } from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

test("a failed restore reports an error and recording policy can be saved", async ({
	page,
	consoleAudit,
}) => {
	let savedPolicy: {
		retention_days: number;
		excluded_channel_ids: string[];
	} | null = null;
	consoleAudit.allow(
		/status of 500.*\/api\/admin\/guilds\/guild-123\/voice-settings/,
	);
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
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
		if (
			path === `/api/admin/guilds/${GUILD_ID}/recording-policy` &&
			request.method() === "GET"
		) {
			await json({
				retention_days: null,
				excluded_channel_ids: [],
				channels: [{ id: "channel-321", name: "Private voice" }],
				is_default: true,
			});
			return;
		}
		if (
			path === `/api/admin/guilds/${GUILD_ID}/recording-policy` &&
			request.method() === "PUT"
		) {
			const body = request.postDataJSON() as NonNullable<typeof savedPolicy>;
			savedPolicy = body;
			await json({
				...body,
				channels: [{ id: "channel-321", name: "Private voice" }],
				is_default: false,
			});
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
	await expect(page.getByRole("alert")).toHaveText(
		"The six-hour default was not restored. The server had a problem (HTTP 500). Try again later.",
	);
	await page
		.getByRole("spinbutton", { name: "Hide recordings after (days)" })
		.fill("30");
	await page.getByRole("checkbox", { name: "Private voice" }).check();
	await page.getByRole("button", { name: "Save recording policy" }).click();
	await expect(page.getByText("Recording policy saved.")).toBeVisible();
	expect(savedPolicy).toEqual({
		retention_days: 30,
		excluded_channel_ids: ["channel-321"],
	});
});

test("server validation and permission failures are explained and input is kept", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/status of (400|403).*\/api\/admin\/guilds\/guild-123\/(recording-policy|voice-settings)/,
	);
	const policyReplies = [
		{
			status: 400,
			json: {
				code: 400,
				kind: "invalid_request",
				message: "Excluded channels must be voice channels in this server",
			},
		},
		{
			status: 403,
			json: {
				code: 403,
				kind: "forbidden",
				message: "You do not have permission to do that.",
			},
		},
	];
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
		const json = (body: unknown, status = 200) =>
			route.fulfill({ status, headers: corsHeaders, json: body });
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
				{ id: GUILD_ID, name: "Test Guild", owner: true, permissions: "8" },
			]);
			return;
		}
		const voiceSettings = `/api/admin/guilds/${GUILD_ID}/voice-settings`;
		if (path === voiceSettings && request.method() === "GET") {
			await json({ pending_cap_seconds: 21_600, is_default: false });
			return;
		}
		if (path === voiceSettings && request.method() === "PUT") {
			await json(
				{
					code: 400,
					kind: "invalid_request",
					message: "The pending recording cap must be at least 60 seconds.",
				},
				400,
			);
			return;
		}
		const policy = `/api/admin/guilds/${GUILD_ID}/recording-policy`;
		if (path === policy && request.method() === "GET") {
			await json({
				retention_days: null,
				excluded_channel_ids: [],
				channels: [{ id: "channel-321", name: "Private voice" }],
				is_default: true,
			});
			return;
		}
		if (path === policy && request.method() === "PUT") {
			const reply = policyReplies.shift();
			await json(reply?.json, reply?.status);
			return;
		}
		await json({ detail: `Unhandled mock route: ${path}` }, 404);
	});

	await page.goto(`/dashboard/${GUILD_ID}/admin/voice-settings`);
	await page
		.getByRole("spinbutton", { name: "Pending cap (seconds)" })
		.fill("600");
	await page.getByRole("button", { name: "Save override" }).click();
	await expect(page.getByRole("alert")).toHaveText(
		"The pending recording cap must be at least 60 seconds.",
	);
	await expect(
		page.getByRole("spinbutton", { name: "Pending cap (seconds)" }),
	).toHaveValue("600");

	const retention = page.getByRole("spinbutton", {
		name: "Hide recordings after (days)",
	});
	await retention.fill("30");
	await page.getByRole("checkbox", { name: "Private voice" }).check();
	const save = page.getByRole("button", { name: "Save recording policy" });
	await save.click();
	const policyAlert = page
		.getByRole("alert")
		.filter({ hasText: "recording policy" })
		.or(page.getByRole("alert").filter({ hasText: "Excluded channels" }));
	await expect(policyAlert).toHaveText(
		"Excluded channels must be voice channels in this server",
	);
	await save.click();
	await expect(policyAlert).toHaveText(
		"The recording policy was not saved. This requires the Manage Server permission.",
	);
	await expect(retention).toHaveValue("30");
	await expect(
		page.getByRole("checkbox", { name: "Private voice" }),
	).toBeChecked();
});
