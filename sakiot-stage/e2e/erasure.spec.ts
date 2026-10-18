import type { Page } from "@playwright/test";
import { expect, fulfillSharedRoutes, test } from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";
const SESSION_ID = "session-123";
const CLIP_ID = "my-voice";
const ME = "current-user";
const PURGE_AT_MS = Date.UTC(2026, 10, 17, 12);
const DELETING_ON = new Date(PURGE_AT_MS).toLocaleDateString(undefined, {
	dateStyle: "long",
});

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

type Erasure = { id: string; state: string; purge_at_ms: number };

/**
 * A member whose own session and clip the server answers for as it would:
 * `removedByServer` starts the session removed by a manager, so the speaker
 * may only erase it. Returns the requests the speaker's actions sent.
 */
async function mockSpeakerApi(
	page: Page,
	options = { removedByServer: false },
) {
	let sessionErasure: Erasure | null = null;
	let clipErasure: Erasure | null = null;
	const sent: string[] = [];
	const erasure = (id: string): Erasure => ({
		id,
		state: "grace",
		purge_at_ms: PURGE_AT_MS,
	});
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname.replace(API_PREFIX, "");
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, corsHeaders)) return;
		const fulfillJson = async (body: unknown, status = 200) => {
			await route.fulfill({
				status,
				headers: { ...corsHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};
		const removal = () =>
			options.removedByServer || sessionErasure
				? {
						erasure: sessionErasure,
						playable: !options.removedByServer,
					}
				: null;
		const status = (erased: Erasure, subject: object) => ({
			id: erased.id,
			state: erased.state,
			requested_at_ms: PURGE_AT_MS - 30 * 24 * 60 * 60 * 1000,
			purge_at_ms: erased.purge_at_ms,
			...subject,
		});

		if (path === "/users/current") {
			await fulfillJson({
				avatar: "",
				is_dev: false,
				user_id: ME,
				username: "Speaker",
			});
			return;
		}
		if (path === "/users/current/guilds") {
			await fulfillJson([
				{ id: GUILD_ID, name: "Test Guild", capabilities: [] },
			]);
			return;
		}
		if (path === `/current/${GUILD_ID}`) {
			await fulfillJson([
				{
					channel_id: "voice-123",
					dirs: [
						{
							year: 2026,
							months: {
								10: [
									{
										file: "1786460400000-Speaker.ogg",
										display_name: "Speaker",
										recording_session_id: SESSION_ID,
										state: "finalized",
										user_id: ME,
										removal: removal(),
									},
								],
							},
						},
					],
				},
			]);
			return;
		}
		if (path === `/current/${GUILD_ID}/live-stems`) {
			await fulfillJson([]);
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/manifest`) {
			if (options.removedByServer) {
				await fulfillJson(
					{ code: 404, kind: "media_not_found", message: "Not found" },
					404,
				);
				return;
			}
			await fulfillJson({
				channel_journey: ["voice-123"],
				current_channel_id: "voice-123",
				duration_ms: 30_000,
				ended_at_ms: 1_786_460_430_000,
				events: [],
				guild_id: GUILD_ID,
				recording_session_id: SESSION_ID,
				segments: [],
				started_at_ms: 1_786_460_400_000,
				starting_channel_id: "voice-123",
				state: "finalized",
				user_id: ME,
			});
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/waveform`) {
			await fulfillJson({ building: false, progress: 100 });
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/remove-silence`) {
			await fulfillJson({ status: "idle", progress: 0 });
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/channel-mix`) {
			await fulfillJson({
				can_generate: false,
				duration_ms: 30_000,
				generation_settings: null,
				media_url: null,
				participants: [],
				progress: 0,
				reason: { code: "no_sources", message: "No mix sources" },
				scope: "all_recordings",
				source_count: 0,
				status: "unavailable",
				tracks: [],
			});
			return;
		}
		if (
			path ===
				`/users/current/guilds/${GUILD_ID}/recordings/${SESSION_ID}/erasure` &&
			request.method() === "POST"
		) {
			sent.push("erase session");
			sessionErasure = erasure("session-erasure");
			await fulfillJson(
				status(sessionErasure, { recording_session_id: SESSION_ID }),
				202,
			);
			return;
		}
		if (
			path === `/users/current/guilds/${GUILD_ID}/clips/${CLIP_ID}/erasure` &&
			request.method() === "POST"
		) {
			sent.push("erase clip");
			clipErasure = erasure("clip-erasure");
			await fulfillJson(status(clipErasure, { clip_id: CLIP_ID }), 202);
			return;
		}
		const undone = /^\/users\/current\/guilds\/[^/]+\/erasures\/([^/]+)$/.exec(
			path,
		);
		const undoneId = undone?.[1];
		if (undoneId && request.method() === "DELETE") {
			sent.push(`undo ${undoneId}`);
			const cancelled =
				undoneId === "clip-erasure" ? clipErasure : sessionErasure;
			if (undoneId === "clip-erasure") clipErasure = null;
			else sessionErasure = null;
			await fulfillJson(
				status({ ...(cancelled ?? erasure(undoneId)), state: "cancelled" }, {}),
			);
			return;
		}
		if (path === `/audio/clips/${GUILD_ID}`) {
			await fulfillJson([
				{
					channel_id: "voice-123",
					clip_id: CLIP_ID,
					guild_id: GUILD_ID,
					length: 2,
					name: "My voice",
					silence_free: false,
					speaker_ids: [ME],
					start_time: 0,
					user_id: "someone-else",
					erasure: clipErasure,
				},
			]);
			return;
		}
		if (path === `/audio/clips/${GUILD_ID}/${CLIP_ID}`) {
			await route.fulfill({
				status: 200,
				headers: { ...corsHeaders, "Content-Type": "audio/wav" },
				body: silentWav(),
			});
			return;
		}
		if (path.startsWith(`/audio/clips/waveform/${GUILD_ID}/`)) {
			await fulfillJson({ progress: 100 });
			return;
		}
		await fulfillJson(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
	return sent;
}

function silentWav(): Buffer {
	const frames = 800;
	const wav = Buffer.alloc(44 + frames * 2);
	wav.write("RIFF", 0);
	wav.writeUInt32LE(36 + frames * 2, 4);
	wav.write("WAVE", 8);
	wav.write("fmt ", 12);
	wav.writeUInt32LE(16, 16);
	wav.writeUInt16LE(1, 20);
	wav.writeUInt16LE(1, 22);
	wav.writeUInt32LE(8_000, 24);
	wav.writeUInt32LE(16_000, 28);
	wav.writeUInt16LE(2, 32);
	wav.writeUInt16LE(16, 34);
	wav.write("data", 36);
	wav.writeUInt32LE(frames * 2, 40);
	return wav;
}

test("a speaker erases their own recording, still plays it, and can undo", async ({
	page,
}) => {
	const sent = await mockSpeakerApi(page);
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	await expect(page.getByRole("tab", { name: "Normal" })).toBeVisible();
	const createClip = page.getByRole("button", {
		name: "Create clip",
		exact: true,
	});
	await expect(createClip).toBeVisible();

	await page.getByRole("button", { name: "Erase permanently" }).click();
	const dialog = page.getByRole("dialog");
	await expect(dialog).toContainText("You can undo this for 30 days");
	await dialog.getByRole("button", { name: "Erase", exact: true }).click();
	await expect(dialog).toHaveCount(0);

	await expect(page.getByText(`Deleting on ${DELETING_ON}.`)).toBeVisible();
	// Still theirs to play, and nothing more.
	await expect(page.getByRole("tab", { name: "Normal" })).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Erase permanently" }),
	).toHaveCount(0);
	await expect(createClip).toHaveCount(0);

	await page.getByRole("button", { name: "Undo" }).click();
	await expect(page.getByText(`Deleting on ${DELETING_ON}.`)).toHaveCount(0);
	await expect(
		page.getByRole("button", { name: "Erase permanently" }),
	).toBeVisible();
	expect(sent).toEqual(["erase session", "undo session-erasure"]);
});

test("a recording the server removed offers its speaker only erasing it", async ({
	page,
	consoleAudit,
}) => {
	// Opened directly, the manifest may be asked for before the tree says
	// the session cannot be played.
	consoleAudit.allow(/status of 404.*\/manifest/);
	const sent = await mockSpeakerApi(page, { removedByServer: true });
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);

	await expect(page.getByText("Removed by the server.")).toBeVisible();
	await expect(page.getByRole("tab", { name: "Normal" })).toHaveCount(0);
	await page.getByRole("button", { name: "Erase permanently" }).click();
	await page
		.getByRole("dialog")
		.getByRole("button", { name: "Erase", exact: true })
		.click();
	await expect(page.getByText(`Deleting on ${DELETING_ON}.`)).toContainText(
		"it cannot be played",
	);

	// Undone, it is removed by the server again, never restored.
	await page.getByRole("button", { name: "Undo" }).click();
	await expect(page.getByText("Removed by the server.")).toBeVisible();
	await expect(page.getByRole("tab", { name: "Normal" })).toHaveCount(0);
	expect(sent).toEqual(["erase session", "undo session-erasure"]);
});

test("a clip of your voice can be erased and the erasure undone", async ({
	page,
}) => {
	const sent = await mockSpeakerApi(page);
	await page.goto(`/dashboard/${GUILD_ID}/clips/${CLIP_ID}`);
	await expect(page.getByRole("button", { name: "Edit clip" })).toBeVisible();

	await page.getByRole("button", { name: "Erase my voice" }).click();
	await page
		.getByRole("dialog")
		.getByRole("button", { name: "Erase", exact: true })
		.click();
	await expect(page.getByText(`Deleting on ${DELETING_ON}.`)).toBeVisible();
	await expect(page.getByRole("button", { name: "Edit clip" })).toHaveCount(0);
	await expect(page.getByRole("button", { name: "Jam It" })).toHaveCount(0);
	await expect(
		page.getByRole("button", { name: "Download clip" }),
	).toBeVisible();

	await page.getByRole("button", { name: "Undo" }).click();
	await expect(
		page.getByRole("button", { name: "Erase my voice" }),
	).toBeVisible();
	expect(sent).toEqual(["erase clip", "undo clip-erasure"]);
});
