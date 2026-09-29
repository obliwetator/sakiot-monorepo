import {
	API_ORIGIN,
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const storageKey = `sakiot:composition-job:current-user:${GUILD_ID}`;
const draft = {
	master_volume_db: 0,
	muted_tracks: [],
	segments: [
		{
			source: "clip",
			source_id: "working-source",
			source_in: 0,
			source_out: 0.1,
			timeline_start: 0,
			track: 0,
			effects: {
				volume_db: 0,
				pitch_cents: 0,
				rate: 1,
				bass_db: 0,
				mid_db: 0,
				treble_db: 0,
				reverse: false,
			},
		},
	],
};

test.beforeEach(async ({ page }) => {
	await mockClipEditorApi(page);
	await page.addInitScript(
		({ draft, guild }) => {
			if (!localStorage.getItem("test:draft-seeded")) {
				localStorage.setItem(
					`sakiot:clip-editor:${guild}:draft`,
					JSON.stringify(draft),
				);
				localStorage.setItem("test:draft-seeded", "yes");
			}
		},
		{ draft, guild: GUILD_ID },
	);
});

test("a queued export reconnects after reload and reaches a stable result", async ({
	page,
}) => {
	let state = "queued";
	let polls = 0;
	await page.addInitScript(
		({ key, body }) => {
			if (!localStorage.getItem("test:job-seeded")) {
				localStorage.setItem(
					key,
					JSON.stringify({ key: "request-key", body, jobId: "job-1" }),
				);
				localStorage.setItem("test:job-seeded", "yes");
			}
		},
		{ key: storageKey, body: draft },
	);
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose/job-1`,
		async (route) => {
			polls++;
			await route.fulfill({
				headers: corsHeaders,
				json: {
					status: state,
					stage: state,
					progress: state === "ready" ? 100 : 0,
					error: null,
					result_clip_id: state === "ready" ? "clip-1" : null,
				},
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await expect(
		page.getByText("Waiting for an available export slot…"),
	).toBeVisible();
	await page.getByRole("button", { name: "Close", exact: true }).click();
	await page.reload();
	await expect.poll(() => polls).toBeGreaterThan(1);
	state = "ready";
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await expect(
		page.getByText("Exported — the new clip is now in the bin."),
	).toBeVisible();
	await expect
		.poll(() => page.evaluate((key) => localStorage.getItem(key), storageKey))
		.toBeNull();
});

test("a lost submission response retries the same request after reload", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/net::ERR_FAILED.*\/api\/audio\/clips\/guild-123\/compose/,
	);
	const requests: { key: string | undefined; body: unknown }[] = [];
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose`,
		async (route) => {
			if (route.request().method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers: corsHeaders });
				return;
			}
			requests.push({
				key: route.request().headers()["idempotency-key"],
				body: route.request().postDataJSON(),
			});
			if (requests.length === 1) {
				await route.abort("failed");
				return;
			}
			await route.fulfill({
				status: 202,
				headers: corsHeaders,
				json: { id: "recovered-job", status: "processing", progress: 0 },
			});
		},
	);
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose/recovered-job`,
		async (route) => {
			await route.fulfill({
				headers: corsHeaders,
				json: {
					status: "ready",
					stage: "ready",
					progress: 100,
					error: null,
					result_clip_id: "recovered-clip",
				},
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await page.getByRole("button", { name: "Render", exact: true }).click();
	await expect(page.getByRole("alert")).toHaveText(
		"Could not confirm whether the export started. The server could not be reached. Check your connection and try again. Try again to check; this cannot create a duplicate export.",
	);
	// The exact request and key survive for a safe resend.
	const kept = await page.evaluate(
		(key) => JSON.parse(localStorage.getItem(key) ?? "null"),
		storageKey,
	);
	expect(requests).toHaveLength(1);
	expect(kept).toMatchObject({ key: requests[0]?.key, jobId: null });
	await page.reload();
	await expect.poll(() => requests.length).toBe(2);
	expect(requests[0]?.key).toBeTruthy();
	expect(requests[1]).toEqual(requests[0]);
	await expect
		.poll(() => page.evaluate((key) => localStorage.getItem(key), storageKey))
		.toBeNull();
});

test("an expired job stops polling and explains how to recover", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(
		/status of 404.*\/api\/audio\/clips\/guild-123\/compose\/expired-job/,
	);
	await page.addInitScript(
		({ key, body }) => {
			localStorage.setItem(
				key,
				JSON.stringify({ key: "expired-request", body, jobId: "expired-job" }),
			);
		},
		{ key: storageKey, body: draft },
	);
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose/expired-job`,
		async (route) => {
			await route.fulfill({
				status: 404,
				headers: corsHeaders,
				json: { code: 404, message: "Clip not found" },
			});
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await expect(
		page.getByText(
			"This export is no longer available. Check your clips before starting another export.",
		),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Render", exact: true }),
	).toBeEnabled();
});

async function routeCompose(
	page: import("@playwright/test").Page,
	replies: { status: number; json: unknown }[],
) {
	const requests: { key: string | undefined; body: unknown }[] = [];
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose`,
		async (route) => {
			if (route.request().method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers: corsHeaders });
				return;
			}
			requests.push({
				key: route.request().headers()["idempotency-key"],
				body: route.request().postDataJSON(),
			});
			const reply = replies.shift() ?? {
				status: 202,
				json: { id: "accepted-job", status: "processing", progress: 0 },
			};
			await route.fulfill({ ...reply, headers: corsHeaders });
		},
	);
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose/accepted-job`,
		(route) =>
			route.fulfill({
				headers: corsHeaders,
				json: {
					status: "queued",
					stage: "queued",
					progress: 0,
					error: null,
					result_clip_id: null,
				},
			}),
	);
	return requests;
}

test("a definite rejection explains itself and keeps the edit", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 503.*\/api\/audio\/clips\/guild-123\/compose/);
	const requests = await routeCompose(page, [
		{
			status: 503,
			json: {
				code: 503,
				kind: "user_job_limit_reached",
				message:
					"You already have the maximum number of active media jobs. Wait for one to finish, then try again.",
			},
		},
	]);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await page.getByLabel("Clip name").fill("My mix");
	await page.getByRole("button", { name: "Render", exact: true }).click();
	await expect(page.getByRole("alert")).toHaveText(
		"The export was not started. You already have the maximum number of active media jobs. Wait for one to finish, then try again.",
	);
	// Nothing was queued, so nothing is tracked; the dialog keeps its input.
	await expect
		.poll(() => page.evaluate((key) => localStorage.getItem(key), storageKey))
		.toBeNull();
	await expect(page.getByLabel("Clip name")).toHaveValue("My mix");
	await page.getByRole("button", { name: "Render", exact: true }).click();
	await expect(
		page.getByText("Waiting for an available export slot…"),
	).toBeVisible();
	expect(requests).toHaveLength(2);
	// A rejected request may be replaced by a fresh one.
	expect(requests[1]?.key).not.toBe(requests[0]?.key);
});

test("an unclassified server fault keeps the request for an identical resend", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 500.*\/api\/audio\/clips\/guild-123\/compose/);
	const requests = await routeCompose(page, [
		{
			status: 500,
			json: {
				code: 500,
				kind: "internal_error",
				message: "Something went wrong on the server.",
			},
		},
	]);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await page.getByRole("button", { name: "Render", exact: true }).click();
	await expect(page.getByRole("alert")).toContainText(
		"Could not confirm whether the export started. Something went wrong on the server.",
	);
	await page.getByRole("button", { name: "Render", exact: true }).click();
	await expect(
		page.getByText("Waiting for an available export slot…"),
	).toBeVisible();
	expect(requests).toHaveLength(2);
	expect(requests[1]).toEqual(requests[0]);
});

test("a failed export reports the server's reason", async ({ page }) => {
	await page.addInitScript(
		({ key, body }) => {
			localStorage.setItem(
				key,
				JSON.stringify({ key: "timed-out", body, jobId: "timed-out-job" }),
			);
		},
		{ key: storageKey, body: draft },
	);
	await page.route(
		`${API_ORIGIN}/api/audio/clips/${GUILD_ID}/compose/timed-out-job`,
		(route) =>
			route.fulfill({
				headers: corsHeaders,
				json: {
					status: "failed",
					stage: "failed",
					progress: 0,
					error: "Processing exceeded its time limit and was stopped.",
					error_kind: "execution_timed_out",
					result_clip_id: null,
				},
			}),
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await page.getByRole("button", { name: "Export", exact: true }).click();
	await expect(page.getByRole("alert")).toHaveText(
		"The export failed. Processing exceeded its time limit and was stopped.",
	);
	await expect(
		page.getByRole("button", { name: "Render", exact: true }),
	).toBeEnabled();
});
