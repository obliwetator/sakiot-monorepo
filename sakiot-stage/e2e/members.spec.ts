import type { Page } from "@playwright/test";
import { expectAccessibleMediaRoute } from "./axe";
import {
	expect,
	fulfillSharedRoutes,
	OWNER_CAPABILITIES,
	test,
} from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
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

/**
 * Plays the members API for a guild whose `@everyone` a Sakiot rule denies
 * seeing General, which Discord lets everyone see and join.
 */
async function mockMembers(page: Page) {
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
		} else if (path === `${base}/${GUILD_ID}/channels`) {
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
						capabilities: [
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
						],
					},
					{
						channel_id: LOUNGE,
						name: "Lounge",
						capabilities: [
							"view_presence",
							"listen_others",
							"create_clips",
							"jam",
							"stamp",
						].map(held),
					},
				],
			});
		} else {
			await json([]);
		}
	});
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
