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
const VIP = "300";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.parse("2026-10-24T12:00:00Z");

interface Removed {
	recording_session_id: string;
	speaker_id: string;
	speaker_name: string | null;
	channel_id: string;
	channel_name: string | null;
	started_at_ms: number;
	ended_at_ms: number;
	removed_at_ms: number;
	reason: "manager" | "retention";
	removed_by?: string;
	redacted: boolean;
	past_retention: boolean;
	restorable: boolean;
}

function removed(
	id: string,
	speaker: string,
	overrides: Partial<Removed> = {},
): Removed {
	return {
		recording_session_id: id,
		speaker_id: `speaker-${id}`,
		speaker_name: speaker,
		channel_id: "100",
		channel_name: "lounge",
		started_at_ms: NOW - 40 * DAY,
		ended_at_ms: NOW - 40 * DAY + 60_000,
		removed_at_ms: NOW - 9 * DAY,
		reason: "retention",
		redacted: false,
		past_retention: true,
		restorable: false,
		...overrides,
	};
}

interface Kept {
	recording_session_id: string;
	audible: boolean;
	speaker_id?: string;
	speaker_name?: string;
	channel_id?: string;
	channel_name?: string;
	started_at_ms?: number;
	mine: boolean;
	keepers: number;
	kept_at_ms: number;
}

interface State {
	capabilities: string[];
	isOperator?: boolean;
	keeping: {
		cap: number;
		kept: number;
		allowance: number | null;
		used: number;
		retention_days: number | null;
		recordings: Kept[];
	};
	/** Each page of removed recordings, in order. */
	pages: Removed[][];
	/** Answers the next keep instead of keeping. */
	refuseKeep?: { status: number; body: unknown };
}

/**
 * Plays the keeping API over `state`: keeping a removed recording moves it
 * to the kept list, restoring one drops it from the removed list, and every
 * write is recorded in `calls`.
 */
async function mockKeeping(page: Page, state: State) {
	const calls: string[] = [];
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname;
		const method = request.method();
		if (method === "OPTIONS") {
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
		const done = () => route.fulfill({ status: 204, headers: corsHeaders });
		if (method !== "GET") calls.push(`${method} ${path}`);

		if (path === "/api/users/current") {
			await json({
				avatar: "",
				is_dev: false,
				is_operator: state.isOperator === true,
				user_id: "current-user",
				username: "Test Member",
			});
			return;
		}
		if (path === "/api/users/current/guilds") {
			await json([
				{
					id: GUILD_ID,
					name: "Test Guild",
					capabilities: state.capabilities,
				},
			]);
			return;
		}
		if (path === `/api/guilds/${GUILD_ID}/kept-recordings`) {
			await json(state.keeping);
			return;
		}
		if (path === `/api/guilds/${GUILD_ID}/removed-recordings`) {
			const before = url.searchParams.get("before");
			const index = before ? Number(before.replace("page-", "")) : 0;
			await json({
				recordings: state.pages[index] ?? [],
				...(index + 1 < state.pages.length
					? { next: `page-${index + 1}` }
					: {}),
			});
			return;
		}
		const keep = path.match(
			/^\/api\/users\/current\/guilds\/[^/]+\/kept-recordings\/([^/]+)$/,
		);
		if (keep && method === "PUT") {
			if (state.refuseKeep) {
				await json(state.refuseKeep.body, state.refuseKeep.status);
				return;
			}
			const id = keep[1] ?? "";
			const found = state.pages
				.flat()
				.find((r) => r.recording_session_id === id);
			state.pages = state.pages.map((page) =>
				page.filter((r) => r.recording_session_id !== id),
			);
			state.keeping.kept += 1;
			state.keeping.used += 1;
			if (found)
				state.keeping.recordings.push({
					recording_session_id: id,
					audible: true,
					speaker_id: found.speaker_id,
					speaker_name: found.speaker_name ?? undefined,
					channel_id: found.channel_id,
					channel_name: found.channel_name ?? undefined,
					started_at_ms: found.started_at_ms,
					mine: true,
					keepers: 1,
					kept_at_ms: NOW,
				});
			await done();
			return;
		}
		if (keep && method === "DELETE") {
			state.keeping.recordings = state.keeping.recordings.filter(
				(r) => r.recording_session_id !== keep[1],
			);
			state.keeping.kept -= 1;
			state.keeping.used -= 1;
			await done();
			return;
		}
		const releaseAll = path.match(
			/^\/api\/admin\/guilds\/[^/]+\/kept-recordings\/([^/]+)$/,
		);
		if (releaseAll && method === "DELETE") {
			state.keeping.recordings = state.keeping.recordings.filter(
				(r) => r.recording_session_id !== releaseAll[1],
			);
			state.keeping.kept -= 1;
			await done();
			return;
		}
		const restore = path.match(
			/^\/api\/admin\/guilds\/[^/]+\/recordings\/([^/]+)\/restore$/,
		);
		if (restore && method === "POST") {
			state.pages = state.pages.map((page) =>
				page.filter((r) => r.recording_session_id !== restore[1]),
			);
			await done();
			return;
		}
		await json({ detail: `Unhandled mock route: ${method} ${path}` }, 404);
	});
	return calls;
}

test("a member keeps a recording retention removed, within their allowance", async ({
	page,
}) => {
	const state: State = {
		capabilities: [],
		keeping: {
			cap: 5,
			kept: 0,
			allowance: 1,
			used: 0,
			retention_days: 30,
			recordings: [],
		},
		pages: [[removed("11", "Ada")], [removed("12", "Grace")]],
	};
	const calls = await mockKeeping(page, state);
	await page.goto(`/dashboard/${GUILD_ID}/kept`);
	await expect(
		page.getByRole("heading", { name: "Kept recordings", exact: true }),
	).toBeVisible();
	await expect(
		page.getByText(
			"Retention removes recordings 30 days after they end. A kept recording stays, for everyone who may hear it, until nobody keeps it.",
		),
	).toBeVisible();
	await expect(
		page.getByText("This server keeps 0 of the 5 recordings it may keep."),
	).toBeVisible();
	await expect(
		page.getByText("You keep 0 of the 1 recording you may keep."),
	).toBeVisible();
	await expect(page.getByText("Nothing is kept yet.")).toBeVisible();
	// A member sees what retention removed, never a restore.
	const removedList = page.getByRole("list", { name: "Removed recordings" });
	await expect(removedList.getByRole("listitem")).toHaveCount(1);
	await expect(page.getByRole("button", { name: /^Restore/ })).toHaveCount(0);
	await page.getByRole("button", { name: "Show more" }).click();
	await expect(removedList.getByRole("listitem")).toHaveCount(2);
	await expect(page.getByRole("button", { name: "Show more" })).toHaveCount(0);

	await page.getByRole("button", { name: /^Keep Ada in lounge/ }).click();
	const keptList = page.getByRole("list", { name: "Kept recordings" });
	await expect(keptList.getByRole("listitem")).toHaveCount(1);
	await expect(keptList.getByText("Kept by you")).toBeVisible();
	await expect(removedList.getByRole("listitem")).toHaveCount(1);
	// The allowance is used up: the rest waits.
	await expect(
		page.getByText(
			"You keep all the recordings you may keep. Stop keeping one to keep another.",
		),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: /^Keep Grace in lounge/ }),
	).toBeDisabled();
	await expectAccessibleMediaRoute(page);

	await page
		.getByRole("button", { name: /^Stop keeping Ada in lounge/ })
		.click();
	await expect(page.getByText("Nothing is kept yet.")).toBeVisible();
	await expect(
		page.getByRole("button", { name: /^Keep Grace in lounge/ }),
	).toBeEnabled();
	expect(calls).toEqual([
		`PUT /api/users/current/guilds/${GUILD_ID}/kept-recordings/11`,
		`DELETE /api/users/current/guilds/${GUILD_ID}/kept-recordings/11`,
	]);
});

test("a manager restores a removal, and a refused keep says why", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 409.*kept-recordings\/22/);
	const state: State = {
		capabilities: OWNER_CAPABILITIES,
		keeping: {
			cap: 2,
			kept: 2,
			allowance: null,
			used: 0,
			retention_days: 30,
			recordings: [
				{
					recording_session_id: "31",
					audible: true,
					speaker_id: "speaker-31",
					speaker_name: "Linus",
					channel_id: "100",
					channel_name: "lounge",
					started_at_ms: NOW - 50 * DAY,
					mine: false,
					keepers: 1,
					kept_at_ms: NOW - DAY,
				},
			],
		},
		pages: [
			[
				removed("21", "Ada", {
					reason: "manager",
					removed_by: "moderator",
					past_retention: false,
					restorable: true,
				}),
				removed("22", "Grace", { redacted: true }),
			],
		],
		refuseKeep: {
			status: 409,
			body: {
				code: 409,
				kind: "conflict",
				message: "This server already keeps 2 recordings, all it may keep.",
			},
		},
	};
	const calls = await mockKeeping(page, state);
	await page.goto(`/dashboard/${GUILD_ID}/kept`);
	const removedList = page.getByRole("list", { name: "Removed recordings" });
	await expect(
		removedList.getByText(/^Removed by a manager on /),
	).toBeVisible();
	await expect(removedList.getByText("Private", { exact: true })).toBeVisible();
	// The cap is full: keeping waits, restoring does not.
	await expect(
		page.getByText("This server keeps all the recordings it may keep."),
	).toBeVisible();
	await page.getByRole("button", { name: /^Restore Ada in lounge/ }).click();
	await expect(removedList.getByRole("listitem")).toHaveCount(1);

	// The server has the last word on a keep.
	state.keeping.kept = 1;
	await page
		.getByRole("button", { name: /^Stop everyone keeping Linus/ })
		.click();
	await expect(page.getByText("Nothing is kept yet.")).toBeVisible();
	await page.getByRole("button", { name: /^Keep Grace in lounge/ }).click();
	await expect(removedList.getByRole("alert")).toHaveText(
		"This server already keeps 2 recordings, all it may keep.",
	);
	expect(calls).toEqual([
		`POST /api/admin/guilds/${GUILD_ID}/recordings/21/restore`,
		`DELETE /api/admin/guilds/${GUILD_ID}/kept-recordings/31`,
		`PUT /api/users/current/guilds/${GUILD_ID}/kept-recordings/22`,
	]);
});

test("a manager shares the server's cap out in allowances", async ({
	page,
}) => {
	const saved: unknown[] = [];
	const reads = heldReads();
	let allowances = [
		{
			subject: { kind: "everyone" },
			name: null,
			allowance: 2,
			reaches_nobody: false,
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
		const json = (body: unknown) =>
			route.fulfill({ status: 200, headers: corsHeaders, json: body });
		if (path === "/api/users/current") {
			await json({
				avatar: "",
				is_dev: false,
				is_operator: false,
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
		if (path === `/api/admin/guilds/${GUILD_ID}/voice-settings`) {
			await json({ pending_cap_seconds: 21_600, is_default: true });
			return;
		}
		if (path === `/api/admin/guilds/${GUILD_ID}/recording-policy`) {
			await json({
				retention_days: 30,
				excluded_channel_ids: [],
				channels: [],
				is_default: false,
			});
			return;
		}
		const allowancesPath = `/api/admin/guilds/${GUILD_ID}/keep-allowances`;
		if (path === allowancesPath && request.method() === "GET") {
			await reads.passed();
			await json({
				cap: 10,
				kept: 3,
				allowances,
				roles: [
					{
						role_id: VIP,
						name: "VIP",
						color: 255,
						color_secondary: null,
						color_tertiary: null,
					},
				],
			});
			return;
		}
		if (path === allowancesPath && request.method() === "PUT") {
			const change = request.postDataJSON() as {
				subject: { kind: string; id?: string };
				allowance: number | null;
			};
			saved.push(change);
			const others = allowances.filter(
				(entry) =>
					entry.subject.kind !== change.subject.kind ||
					(entry.subject as { id?: string }).id !== change.subject.id,
			);
			allowances =
				change.allowance === null
					? others
					: [
							...others,
							{
								subject: change.subject,
								name: null,
								allowance: change.allowance,
								reaches_nobody: false,
							},
						];
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		await route.fulfill({
			status: 404,
			headers: corsHeaders,
			json: { detail: `Unhandled mock route: ${request.method()} ${path}` },
		});
	});

	await page.goto(`/dashboard/${GUILD_ID}/admin/voice-settings`);
	const panel = page.getByRole("list", { name: "Allowances" });
	await expect(
		page.getByText(/This server may keep 10 recordings and keeps 3/),
	).toBeVisible();
	const everyone = panel.getByRole("spinbutton", { name: "@everyone" });
	await expect(everyone).toHaveValue("2");
	// Saved, it shows what was saved while the read confirming it is slow.
	reads.hold();
	await everyone.fill("3");
	await panel.getByRole("button", { name: "Save" }).first().click();
	await expect(panel.getByText("Allowance saved.")).toBeVisible();
	await expect(
		panel.getByRole("button", { name: "Save" }).first(),
	).toBeDisabled();
	await expect(everyone).toHaveValue("3");
	reads.release();

	await page.getByRole("button", { name: /Add a role/ }).click();
	await page.getByRole("option", { name: "VIP" }).click();
	const vip = panel.getByRole("spinbutton", { name: "@VIP" });
	await vip.fill("-1");
	await expect(
		panel.getByText("A whole number from 0 to 100000, or blank."),
	).toBeVisible();
	await vip.fill("5");
	await panel
		.getByRole("listitem")
		.filter({ has: page.getByRole("spinbutton", { name: "@VIP" }) })
		.getByRole("button", { name: "Save" })
		.click();
	await expect(
		panel.getByRole("button", { name: "Remove the allowance for @VIP" }),
	).toBeVisible();
	await panel
		.getByRole("button", { name: "Remove the allowance for @everyone" })
		.click();
	await expect(everyone).toHaveValue("");
	expect(saved).toEqual([
		{ subject: { kind: "everyone" }, allowance: 3 },
		{ subject: { kind: "role", id: VIP }, allowance: 5 },
		{ subject: { kind: "everyone" }, allowance: null },
	]);
	await expectAccessibleMediaRoute(page);
});

test("an operator sets how many recordings a server may keep", async ({
	page,
}) => {
	const limits: unknown[] = [];
	const reads = heldReads();
	let cap = 0;
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
		const json = (body: unknown) =>
			route.fulfill({ status: 200, headers: corsHeaders, json: body });
		if (path === "/api/users/current") {
			await json({
				avatar: "",
				is_dev: false,
				is_operator: true,
				user_id: "current-user",
				username: "Operator",
			});
			return;
		}
		if (path === "/api/users/current/guilds") {
			await json([]);
			return;
		}
		if (path === "/api/operator/guilds") {
			await reads.passed();
			await json([
				{
					guild_id: GUILD_ID,
					name: "Test Guild",
					icon: null,
					present: true,
					kept_recordings_cap: cap,
					kept_recordings: 4,
					retention_days: 30,
				},
				{
					guild_id: "guild-gone",
					name: null,
					icon: null,
					present: false,
					kept_recordings_cap: 0,
					kept_recordings: 0,
					retention_days: null,
				},
			]);
			return;
		}
		if (path === `/api/operator/guilds/${GUILD_ID}/limits`) {
			const body = request.postDataJSON() as { kept_recordings: number };
			limits.push(body);
			cap = body.kept_recordings;
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		await route.fulfill({
			status: 404,
			headers: corsHeaders,
			json: { detail: `Unhandled mock route: ${request.method()} ${path}` },
		});
	});

	await page.goto("/operator");
	await expect(
		page.getByRole("heading", { name: "Operator", exact: true }),
	).toBeVisible();
	const guild = page.getByRole("listitem", { name: "Test Guild" });
	await expect(guild.getByText(/^Keeps 4 of 0 recordings/)).toBeVisible();
	await expect(
		page
			.getByRole("listitem", { name: "Server guild-gone" })
			.getByText("Bot not in it"),
	).toBeVisible();
	const field = guild.getByRole("spinbutton", {
		name: "Recordings it may keep",
	});
	await field.fill("2");
	await expect(
		guild.getByText(
			"Below what it keeps: those stay kept, and nothing more can be until it is under the cap.",
		),
	).toBeVisible();
	reads.hold();
	await field.fill("25");
	await guild.getByRole("button", { name: "Save" }).click();
	await expect(guild.getByText("Saved.")).toBeVisible();
	await expect(guild.getByRole("button", { name: "Save" })).toBeDisabled();
	await expect(field).toHaveValue("25");
	await expect(guild.getByText(/^Keeps 4 of 25 recordings/)).toBeVisible();
	reads.release();
	expect(limits).toEqual([{ kept_recordings: 25 }]);
	await expectAccessibleMediaRoute(page);

	// Operators reach the page from their account menu.
	await page.getByRole("button", { name: "Open user menu" }).click();
	await expect(
		page.getByRole("menuitem", { name: "Operator dashboard" }),
	).toBeVisible();
});
