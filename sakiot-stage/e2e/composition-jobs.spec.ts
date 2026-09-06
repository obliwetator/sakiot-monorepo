import { expect, test } from "@playwright/test";
import {
	API_ORIGIN,
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";

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
}) => {
	const requests: { key: string; body: unknown }[] = [];
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
	await expect(
		page.getByText(
			"Could not confirm the export. Press Render to retry safely.",
		),
	).toBeVisible();
	await page.reload();
	await expect.poll(() => requests.length).toBe(2);
	expect(requests[0].key).toBeTruthy();
	expect(requests[1]).toEqual(requests[0]);
	await expect
		.poll(() => page.evaluate((key) => localStorage.getItem(key), storageKey))
		.toBeNull();
});

test("an expired job stops polling and explains how to recover", async ({
	page,
}) => {
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
