import type { Locator, Page } from "@playwright/test";
import {
	expect,
	fulfillSharedRoutes,
	OWNER_CAPABILITIES,
	test,
} from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

const ROLES = Array.from({ length: 40 }, (_, index) => ({
	role_id: `role-${index}`,
	name: `Role ${index}`,
	permission: "1049600",
	member_count: 200,
	color: 0,
	color_secondary: null,
	color_tertiary: null,
}));

const MEMBERS = Array.from({ length: 200 }, (_, index) => ({
	user_id: `${1000 + index}`,
	name: `Member ${index}`,
}));

const STAMPS = Array.from({ length: 120 }, (_, index) => ({
	audio_file_id: index,
	channel_id: "channel-123",
	channel_name: "General",
	created_at: "2026-08-11T12:30:00.000Z",
	guild_id: GUILD_ID,
	id: 9000 + index,
	note: `Stamp note ${index}`,
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
}));

const BOT_LOG = Array.from({ length: 100 }, (_, index) => ({
	id: index,
	kind: "deafened",
	occurred_at_ms: 1_786_460_400_000 - index * 60_000,
	channel_name: `Channel ${index}`,
	to_channel_name: null,
	actor_id: "10001",
	actor_name: "A moderator",
	attribution: "exact",
}));

/** Answers with more of everything than any screen can show at once. */
async function mockLongLists(page: Page) {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		const json = (body: unknown, status = 200) =>
			route.fulfill({
				status,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		if (path === `/api/admin/guilds/${GUILD_ID}/bot-log`) {
			await json(BOT_LOG);
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
		const roles = `/api/admin/guilds/${GUILD_ID}/roles`;
		if (path === "/api/users/current") {
			await json({
				avatar: "",
				is_dev: false,
				user_id: "current-user",
				username: "Test Admin",
			});
		} else if (path === "/api/users/current/guilds") {
			await json([
				{ id: GUILD_ID, name: "Test Guild", capabilities: OWNER_CAPABILITIES },
			]);
		} else if (path === roles) {
			await json(ROLES);
		} else if (path.startsWith(`${roles}/`) && path.endsWith("/members")) {
			await json(MEMBERS);
		} else if (path === `/api/stamps/${GUILD_ID}`) {
			await json(STAMPS);
		} else if (path === `/api/admin/guilds/${GUILD_ID}/voice-settings`) {
			await json({ pending_cap_seconds: 21_600, is_default: true });
		} else if (path === `/api/admin/guilds/${GUILD_ID}/keep-allowances`) {
			await json({ cap: 0, kept: 0, allowances: [], roles: [] });
		} else if (path === `/api/admin/guilds/${GUILD_ID}/recording-policy`) {
			await json({ excluded_channel_ids: [], channels: [], is_default: true });
		} else {
			await json({ detail: `Unhandled mock route: ${path}` }, 404);
		}
	});
}

/** Scrolls `region` to its end and returns how far it moved. */
async function scrollToEnd(region: Locator) {
	return region.evaluate((element) => {
		element.scrollTop = element.scrollHeight;
		return element.scrollTop;
	});
}

async function expectPageStayedPut(page: Page) {
	expect(
		await page.evaluate(() => ({
			document: document.scrollingElement?.scrollTop,
			frame: document.querySelector("[data-testid=page-frame]")?.scrollTop,
		})),
	).toEqual({ document: 0, frame: 0 });
}

test("only the members scroll, not the page", async ({ page }) => {
	await mockLongLists(page);
	await page.goto(`/dashboard/${GUILD_ID}/members`);

	const title = page.getByRole("heading", { name: "Members & roles" });
	const members = page.getByRole("region", { name: "Members", exact: true });
	const roles = page.getByRole("region", { name: "Roles" });
	const columnName = members.getByRole("columnheader", { name: "Name" });
	await expect(members.getByRole("cell", { name: "Member 0" })).toBeVisible();

	expect(await scrollToEnd(members)).toBeGreaterThan(0);
	await expect(
		members.getByRole("cell", { name: "Member 199", exact: true }),
	).toBeInViewport();
	await expect(title).toBeInViewport();
	await expect(columnName).toBeInViewport();
	// Scrolling the members leaves the roles where they were.
	expect(await roles.evaluate((element) => element.scrollTop)).toBe(0);

	expect(await scrollToEnd(roles)).toBeGreaterThan(0);
	await expect(
		roles.getByRole("button", { name: "Role 39", exact: false }).first(),
	).toBeInViewport();
	await expect(title).toBeInViewport();
	await expectPageStayedPut(page);
});

test("keyboards can scroll the members", async ({ page }) => {
	await mockLongLists(page);
	await page.goto(`/dashboard/${GUILD_ID}/members`);
	const members = page.getByRole("region", { name: "Members", exact: true });
	await expect(members.getByRole("cell", { name: "Member 0" })).toBeVisible();

	await members.focus();
	await page.keyboard.press("End");
	await expect
		.poll(() => members.evaluate((element) => element.scrollTop))
		.toBeGreaterThan(0);
});

test("only the stamps scroll, not the page", async ({ page }) => {
	await mockLongLists(page);
	await page.goto(`/stamps/${GUILD_ID}`);

	const stamps = page.getByRole("region", { name: "Stamps" });
	await expect(stamps.getByRole("cell", { name: "9000" })).toBeVisible();

	expect(await scrollToEnd(stamps)).toBeGreaterThan(0);
	await expect(
		stamps.getByRole("cell", { name: "9119", exact: true }),
	).toBeInViewport();
	await expect(
		stamps.getByRole("columnheader", { name: "ID", exact: true }),
	).toBeInViewport();
	await expect(page.getByRole("heading", { name: /^Stamps/ })).toBeInViewport();
	await expectPageStayedPut(page);
});

test("the bot log scrolls by itself under a title that stays", async ({
	page,
}) => {
	await mockLongLists(page);
	await page.goto(`/dashboard/${GUILD_ID}/admin/voice-settings`);

	const title = page.getByRole("heading", { name: "Voice Settings" });
	const log = page.getByRole("region", { name: "Bot log, scrollable" });
	await expect(log).toBeVisible();

	// The settings column reaches the log, which then scrolls on its own.
	await log.scrollIntoViewIfNeeded();
	await expect(title).toBeInViewport();
	expect(await scrollToEnd(log)).toBeGreaterThan(0);
	await expect(log.getByText("Channel 99", { exact: false })).toBeInViewport();
	await expectPageStayedPut(page);
});

test("the logged-out message is on screen", async ({ page, consoleAudit }) => {
	consoleAudit.allow(/status of 401.*\/api\//);
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		await route.fulfill({ status: 401, headers: corsHeaders, body: "{}" });
	});
	await page.goto("/dashboard");
	await expect(page.getByText("You are not logged in")).toBeInViewport();
});
