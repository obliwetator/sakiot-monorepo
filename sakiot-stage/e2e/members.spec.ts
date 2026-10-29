import type { Page, WebSocketRoute } from "@playwright/test";
import { expectAccessibleMediaRoute } from "./axe";
import {
	expect,
	fulfillSharedRoutes,
	OWNER_CAPABILITIES,
	test,
} from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const SOCKET_URL = "ws://127.0.0.1:4174/api/realtime";
const GUILD_ID = "guild-123";
const GENERAL = "100";
const LOUNGE = "200";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

const fromDiscord = { kind: "discord", channel_id: null, subject: null };
const unseen = { kind: "unseen", channel_id: null, subject: null };
const held = (capability: string) => ({
	capability,
	held: true,
	discord: true,
	source: fromDiscord,
});

const CONTENT = [
	"view_presence",
	"listen_others",
	"create_clips",
	"jam",
	"stamp",
];

/**
 * Plays the members API for a guild whose `@everyone` a Sakiot rule denies
 * seeing General, which Discord lets everyone see and join, and the rules
 * API, where saving General's `@everyone` rule replaces it.
 */
async function mockMembers(page: Page) {
	const general = {
		rule: { allow: [] as string[], deny: ["view_presence"] },
		roleViews: 0,
	};
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
		const json = async (body: unknown) => {
			await route.fulfill({
				status: 200,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};
		const base = `/api/admin/guilds/${GUILD_ID}/roles`;
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
		} else if (path === base) {
			await json([
				{
					role_id: GUILD_ID,
					name: "@everyone",
					permission: "1049600",
					member_count: 3,
					color: 0,
					color_secondary: null,
					color_tertiary: null,
				},
			]);
		} else if (path === `${base}/${GUILD_ID}/members`) {
			await json([]);
		} else if (path === `/api/admin/guilds/${GUILD_ID}/permission-rules`) {
			if (request.method() === "PUT") {
				general.rule = request.postDataJSON();
				await route.fulfill({ status: 204, headers: corsHeaders });
				return;
			}
			await json({
				editable: [...OWNER_CAPABILITIES, ...CONTENT],
				channels: [
					{ channel_id: GENERAL, name: "General", editable: CONTENT },
					{ channel_id: LOUNGE, name: "Lounge", editable: CONTENT },
				],
				roles: [],
				rules: [
					{
						channel_id: GENERAL,
						subject: { kind: "everyone" },
						...general.rule,
						name: null,
						reaches_nobody: false,
						updated_by: "current-user",
						updated_at: "2026-10-29T10:00:00Z",
					},
				],
			});
		} else if (path === `${base}/${GUILD_ID}/channels`) {
			general.roleViews += 1;
			const hidden = general.rule.deny.includes("view_presence");
			await json({
				guild: OWNER_CAPABILITIES.map((capability) => ({
					capability,
					held: false,
					discord: false,
					source: fromDiscord,
				})),
				channels: [
					{
						channel_id: GENERAL,
						name: "General",
						capabilities: hidden
							? [
									{
										capability: "view_presence",
										held: false,
										discord: true,
										source: {
											kind: "rule",
											channel_id: GENERAL,
											subject: { kind: "everyone" },
										},
									},
									...["listen_others", "create_clips", "jam", "stamp"].map(
										(capability) => ({
											capability,
											held: false,
											discord: true,
											source: unseen,
										}),
									),
								]
							: CONTENT.map(held),
					},
					{
						channel_id: LOUNGE,
						name: "Lounge",
						capabilities: CONTENT.map(held),
					},
				],
			});
		} else {
			await json([]);
		}
	});
	return general;
}

/** Opens a page from the navigation, as a member would: no reload. */
async function navigateTo(page: Page, name: string) {
	const openNavigation = page.getByRole("button", { name: "open navigation" });
	if (await openNavigation.isVisible()) await openNavigation.click();
	await page.getByRole("button", { name, exact: true }).click();
}

test("a role preview says a Sakiot rule hides a channel Discord shows", async ({
	page,
}) => {
	await mockMembers(page);
	await page.goto(`/dashboard/${GUILD_ID}/members`);
	await page.getByRole("button", { name: "View as @everyone" }).click();
	const dialog = page.getByRole("dialog");

	const general = dialog.getByRole("listitem", { name: "General" });
	await expect(general.getByText("Hidden", { exact: true })).toBeVisible();
	await expect(
		general.getByText(
			"See who is in voice: denied by a Sakiot rule for @everyone in this channel. Discord alone would allow it.",
		),
	).toBeVisible();
	await expect(
		general.getByText(
			"Listen to others, Make clips, Play clips in voice and Stamp: not allowed, because the role can't see this channel.",
		),
	).toBeVisible();

	// Discord decided everything in the lounge: nothing to explain.
	const lounge = dialog.getByRole("listitem", { name: "Lounge" });
	await expect(lounge.getByText("Can listen", { exact: true })).toBeVisible();
	await expect(lounge.getByText(/Sakiot rule/)).toHaveCount(0);
	await expectAccessibleMediaRoute(page);
});

test("a rule saved on the Permissions page shows in the next role preview", async ({
	page,
}) => {
	const general = await mockMembers(page);
	await page.goto(`/dashboard/${GUILD_ID}/members`);
	await page.getByRole("button", { name: "View as @everyone" }).click();
	const dialog = page.getByRole("dialog");
	await expect(
		dialog
			.getByRole("listitem", { name: "General" })
			.getByText("Hidden", { exact: true }),
	).toBeVisible();
	await dialog.getByRole("button", { name: "Close" }).click();

	// Allow @everyone to see General again.
	await navigateTo(page, "Permissions");
	await page.getByRole("button", { name: /Where/ }).click();
	await page.getByRole("option", { name: "General" }).click();
	const rule = page.getByRole("region", { name: "Rule for @everyone" });
	await rule
		.getByRole("radiogroup", { name: "See who is in voice for @everyone" })
		.getByText("Allow", { exact: true })
		.click();
	await rule.getByRole("button", { name: "Save rule" }).click();
	await expect(rule.getByText("Rule saved.")).toBeVisible();

	// Back within the cache's lifetime, the preview asks again.
	await navigateTo(page, "Members");
	await page.getByRole("button", { name: "View as @everyone" }).click();
	await expect(
		dialog
			.getByRole("listitem", { name: "General" })
			.getByText("Can listen", { exact: true }),
	).toBeVisible();
	expect(general.roleViews).toBe(2);
});

test("a Discord permission change refreshes an open role preview", async ({
	page,
}) => {
	const general = await mockMembers(page);
	let socket: WebSocketRoute | null = null;
	await page.routeWebSocket(SOCKET_URL, (ws) => {
		socket = ws;
		const now = Date.now();
		ws.send(
			JSON.stringify({
				type: "ready",
				v: 1,
				user_id: "current-user",
				server_time: now,
				token_expires_at: now + 15 * 60_000,
			}),
		);
		ws.onMessage((raw) => {
			const message = JSON.parse(String(raw)) as {
				type: string;
				guild_id?: string;
			};
			if (message.type !== "set_scope") return;
			ws.send(
				JSON.stringify({
					type: "subscribed",
					v: 1,
					guild_id: message.guild_id,
				}),
			);
		});
	});
	await page.goto(`/dashboard/${GUILD_ID}/members`);
	await page.getByRole("button", { name: "View as @everyone" }).click();
	const generalRow = page
		.getByRole("dialog")
		.getByRole("listitem", { name: "General" });
	await expect(generalRow.getByText("Hidden", { exact: true })).toBeVisible();
	await expect.poll(() => socket !== null).toBe(true);

	// Discord now shows General to everyone; the server says so.
	general.rule = { allow: [], deny: [] };
	(socket as unknown as WebSocketRoute).send(
		JSON.stringify({
			type: "changed",
			v: 1,
			guild_id: GUILD_ID,
			resource: "permissions",
		}),
	);
	await expect(
		generalRow.getByText("Can listen", { exact: true }),
	).toBeVisible();
});
