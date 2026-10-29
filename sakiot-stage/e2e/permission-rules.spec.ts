import type { Page } from "@playwright/test";
import { expectAccessibleMediaRoute } from "./axe";
import {
	expect,
	fulfillSharedRoutes,
	heldReads,
	OWNER_CAPABILITIES,
	test,
} from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const GUILD_ID = "guild-123";
const LOUNGE = "100";
const STAFF = "200";
const VIP = "300";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

interface Rule {
	channel_id: string | null;
	subject: { kind: string; id?: string };
	allow: string[];
	deny: string[];
}

/**
 * Plays the rules API: the editor holds everything in the lounge and
 * nothing in the staff channel; saved rules come back on the next read,
 * which `reads` may hold, and `refusals` answer the next PUTs instead of
 * saving them.
 */
async function mockRules(
	page: Page,
	refusals: { status: number; body: unknown }[] = [],
) {
	const rules: Rule[] = [
		// A rule whose role is gone.
		{
			channel_id: null,
			subject: { kind: "role", id: "999" },
			allow: ["stamp"],
			deny: [],
		},
	];
	const saved: Rule[] = [];
	const explained: string[] = [];
	const reads = heldReads();
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname;
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
				{ id: GUILD_ID, name: "Test Guild", capabilities: OWNER_CAPABILITIES },
			]);
			return;
		}
		const base = `/api/admin/guilds/${GUILD_ID}/permission-rules`;
		if (path === base && request.method() === "GET") {
			await reads.passed();
			await json({
				editable: [...OWNER_CAPABILITIES, "stamp"],
				channels: [
					{
						channel_id: LOUNGE,
						name: "lounge",
						editable: [
							"view_presence",
							"listen_others",
							"create_clips",
							"jam",
							"stamp",
						],
					},
					{ channel_id: STAFF, name: "staff", editable: [] },
				],
				roles: [
					{
						role_id: VIP,
						name: "VIP",
						color: 255,
						color_secondary: null,
						color_tertiary: null,
					},
				],
				rules: rules.map((rule) => ({
					...rule,
					name: null,
					reaches_nobody: rule.subject.id === "999",
					updated_by: "current-user",
					updated_at: "2026-10-22T10:00:00Z",
				})),
			});
			return;
		}
		if (path === base && request.method() === "PUT") {
			const refusal = refusals.shift();
			if (refusal) {
				await json(refusal.body, refusal.status);
				return;
			}
			const rule = request.postDataJSON() as Rule;
			saved.push(rule);
			const same = (other: Rule) =>
				other.channel_id === rule.channel_id &&
				other.subject.kind === rule.subject.kind &&
				other.subject.id === rule.subject.id;
			const kept = rules.filter((other) => !same(other));
			rules.splice(0, rules.length, ...kept);
			if (rule.allow.length > 0 || rule.deny.length > 0) rules.push(rule);
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (path === `${base}/explain`) {
			explained.push(url.search);
			const decision = (capability: string, held: boolean) => ({
				capability,
				held,
				discord: true,
				source:
					capability === "jam"
						? {
								kind: "rule",
								channel_id: LOUNGE,
								subject: { kind: "role", id: VIP },
							}
						: { kind: "discord", channel_id: null, subject: null },
			});
			await json({
				membership: "member",
				guild: [],
				channels: [
					{
						channel_id: LOUNGE,
						name: "lounge",
						capabilities: [
							decision("view_presence", true),
							decision("listen_others", true),
							decision("create_clips", true),
							decision("jam", false),
							decision("stamp", true),
						],
					},
				],
			});
			return;
		}
		await json(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
	return { saved, explained, reads };
}

test("an editor denies a role a capability in one channel and sees what it decides", async ({
	page,
}) => {
	const { saved, explained } = await mockRules(page);
	await page.goto(`/dashboard/${GUILD_ID}/admin/permissions`);
	await expect(
		page.getByRole("heading", { name: "Permissions", exact: true }),
	).toBeVisible();

	// The lounge, for a role that has no rule there yet.
	await page.getByRole("button", { name: /Where/ }).click();
	await page.getByRole("option", { name: "lounge" }).click();
	await page.getByRole("button", { name: /Add a role/ }).click();
	await page.getByRole("option", { name: "VIP" }).click();
	const rule = page.getByRole("region", { name: "Rule for @VIP" });
	await expect(rule).toBeVisible();
	// Each capability names the Discord permissions that decide it otherwise.
	await expect(
		rule.getByText("Without a rule: Discord's View Channel and Connect.", {
			exact: true,
		}),
	).toHaveCount(3);
	await expect(
		rule.getByText("Without a rule: Discord's View Channel.", { exact: true }),
	).toHaveCount(2);
	// Channels hold content capabilities only.
	await expect(
		rule.getByRole("radiogroup", { name: "Manage settings for @VIP" }),
	).toHaveCount(0);
	// React Aria hides the input itself; its label is what one clicks.
	await rule
		.getByRole("radiogroup", { name: "Play clips in voice for @VIP" })
		.getByText("Deny", { exact: true })
		.click();
	await rule.getByRole("button", { name: "Save rule" }).click();
	await expect(rule.getByText("Rule saved.")).toBeVisible();
	expect(saved).toEqual([
		{
			channel_id: LOUNGE,
			subject: { kind: "role", id: VIP },
			allow: [],
			deny: ["jam"],
		},
	]);
	// Saved, so nothing is left to save.
	await expect(rule.getByRole("button", { name: "Save rule" })).toBeDisabled();
	await expect(
		page
			.getByRole("list", { name: "Rules here" })
			.getByRole("button", { name: "VIP" }),
	).not.toContainText("no rule");

	// What the role holds there, and the rule that decided it.
	await page.getByRole("button", { name: /A role/ }).click();
	await page.getByRole("option", { name: "@VIP" }).click();
	const access = page.getByRole("region", { name: "Access of @VIP" });
	await expect(
		access.getByRole("cell", { name: /Denied.*Denied for @VIP in lounge/ }),
	).toBeVisible();
	expect(explained).toEqual([`?role_id=${VIP}`]);
	await expectAccessibleMediaRoute(page);
});

test("a saved rule shows as saved while the page reads it back", async ({
	page,
}) => {
	const { reads } = await mockRules(page);
	await page.goto(`/dashboard/${GUILD_ID}/admin/permissions`);
	await page.getByRole("button", { name: /Where/ }).click();
	await page.getByRole("option", { name: "lounge" }).click();
	const rule = page.getByRole("region", { name: "Rule for @everyone" });
	const jam = rule.getByRole("radiogroup", {
		name: "Play clips in voice for @everyone",
	});
	const everyone = page
		.getByRole("list", { name: "Rules here" })
		.getByRole("button", { name: "@everyone" });
	await jam.getByText("Deny", { exact: true }).click();
	await rule.getByRole("button", { name: "Save rule" }).click();
	await expect(everyone).not.toContainText("no rule");

	// Back to the default, while the read that would confirm it is slow.
	reads.hold();
	await jam.getByText("Default", { exact: true }).click();
	await rule.getByRole("button", { name: "Save rule" }).click();
	await expect(rule.getByText("Rule saved.")).toBeVisible();
	// The draft is gone, and what shows is the rule just saved.
	await expect(rule.getByRole("button", { name: "Save rule" })).toBeDisabled();
	await expect(jam.getByRole("radio", { name: "Default" })).toBeChecked();
	await expect(everyone).toContainText("no rule");
	reads.release();
	await expect(jam.getByRole("radio", { name: "Default" })).toBeChecked();
});

test("what the editor lacks stays locked, and refusals and stale rules are explained", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/status of 403.*\/api\/admin\/guilds\/guild-123\/permission-rules/,
	);
	const { saved } = await mockRules(page, [
		{
			status: 403,
			body: {
				code: 403,
				kind: "forbidden",
				message: "This would take away your own permission to edit rules.",
			},
		},
	]);
	await page.goto(`/dashboard/${GUILD_ID}/admin/permissions`);

	// The staff channel's capabilities are not the editor's to change.
	await page.getByRole("button", { name: /Where/ }).click();
	await page.getByRole("option", { name: "staff" }).click();
	const everyone = page.getByRole("region", { name: "Rule for @everyone" });
	await expect(
		everyone
			.getByRole("radiogroup", { name: "Listen to others for @everyone" })
			.getByRole("radio", { name: "Deny" }),
	).toBeDisabled();

	// Server-wide, the server says why it refuses.
	await page.getByRole("button", { name: /Where/ }).click();
	await page.getByRole("option", { name: "Whole server" }).click();
	await everyone
		.getByRole("radiogroup", { name: "Edit permissions for @everyone" })
		.getByText("Deny", { exact: true })
		.click();
	await everyone.getByRole("button", { name: "Save rule" }).click();
	await expect(everyone.getByRole("alert")).toHaveText(
		"This would take away your own permission to edit rules.",
	);
	await expect(
		everyone.getByRole("button", { name: "Save rule" }),
	).toBeEnabled();

	// A rule whose role is gone can only go.
	await page
		.getByRole("button", {
			name: "Delete the rule for Deleted role 999, server-wide",
		})
		.click();
	await expect(page.getByText("These rules reach nobody")).toHaveCount(0);
	expect(saved).toEqual([
		{
			channel_id: null,
			subject: { kind: "role", id: "999" },
			allow: [],
			deny: [],
		},
	]);
});
