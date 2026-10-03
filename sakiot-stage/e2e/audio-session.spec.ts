import type { Page } from "@playwright/test";
import { expectAccessibleMediaRoute } from "./axe";
import { expect, fulfillSharedRoutes, test } from "./fixtures";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";
const SESSION_ID = "session-123";
const RECORDING_FILE = "1786460400000-Test_User.ogg";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

test("audio session route passes Axe", async ({ page }) => {
	await mockAudioApi(page);
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	await expect(page.getByRole("tab", { name: "Normal" })).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Audio", exact: true }),
	).toHaveAttribute("aria-current", "page");
	await expectAccessibleMediaRoute(page);
});

test("recording removal is soft and explains retained media", async ({
	page,
}) => {
	await mockAudioApi(page);
	page.on("dialog", (dialog) => dialog.accept());
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	const request = page.waitForRequest(
		(url) =>
			url.method() === "DELETE" &&
			url
				.url()
				.endsWith(`/api/admin/guilds/${GUILD_ID}/recordings/${SESSION_ID}`),
	);
	await page
		.getByRole("button", { name: "Remove recording from view" })
		.click();
	await request;
	await expect(
		page.getByText(
			"Recording removed from view. Its media and metadata are retained.",
		),
	).toBeVisible();
});

test("a removal status that cannot be loaded never claims deletion progress", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 500.*\/recording-deletions\/soft-job/);
	await mockAudioApi(page);
	let statusAvailable = false;
	await page.route(
		`${API_ORIGIN}${API_PREFIX}/admin/guilds/${GUILD_ID}/recording-deletions/soft-job`,
		async (route, request) => {
			if (statusAvailable) {
				await route.fallback();
				return;
			}
			expect(request.method()).toBe("GET");
			await route.fulfill({ status: 500, headers: corsHeaders, body: "" });
		},
	);
	page.on("dialog", (dialog) => dialog.accept());
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	await page
		.getByRole("button", { name: "Remove recording from view" })
		.click();
	// The removal reply itself confirmed the soft deletion.
	await expect(
		page.getByText(
			"Recording removed from view. Its media and metadata are retained.",
		),
	).toBeVisible();
	await expect(page.getByRole("alert")).toContainText(
		"Could not refresh the deletion status; showing the last known state.",
	);
	await expect(page.getByText(/in progress|permanently deleted/)).toHaveCount(
		0,
	);
	statusAvailable = true;
	await page.getByRole("button", { name: "Check again" }).click();
	await expect(page.getByRole("alert")).toHaveCount(0);
	await expect(page.getByRole("button", { name: "Check again" })).toHaveCount(
		0,
	);
});

test("removal conflicts and missing permissions are explained", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of (409|403).*\/recordings\/session-123/);
	await mockAudioApi(page);
	const replies = [
		{
			status: 409,
			json: {
				code: 409,
				kind: "conflict",
				message: "Only finalized recordings can be deleted",
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
	await page.route(
		`${API_ORIGIN}${API_PREFIX}/admin/guilds/${GUILD_ID}/recordings/${SESSION_ID}`,
		async (route, request) => {
			if (request.method() !== "DELETE") {
				await route.fallback();
				return;
			}
			const reply = replies.shift();
			await route.fulfill({ ...reply, headers: corsHeaders });
		},
	);
	page.on("dialog", (dialog) => dialog.accept());
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	const remove = page.getByRole("button", {
		name: "Remove recording from view",
	});
	await remove.click();
	await expect(page.getByRole("alert")).toHaveText(
		"Could not remove the recording. Only finalized recordings can be deleted",
	);
	await remove.click();
	await expect(page.getByRole("alert")).toHaveText(
		"Removing recordings requires the Manage Server permission.",
	);
});

test("native controls keep focus styling, pseudo-elements, and responsive layouts", async ({
	page,
}, testInfo) => {
	await mockAudioApi(page);
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	const tab = page.getByRole("tab", { name: "Normal", exact: true });
	await expect(tab).toHaveAttribute("aria-selected", "true");
	const panel = page.getByRole("tabpanel", { name: "Normal", exact: true });
	await expect(panel).toBeVisible();
	expect(await tab.getAttribute("aria-controls")).toBe(
		await panel.getAttribute("id"),
	);
	expect(await panel.getAttribute("aria-labelledby")).toBe(
		await tab.getAttribute("id"),
	);

	const position = page.getByRole("slider", {
		name: "Logical playback position",
	});
	await page.keyboard.press("Tab");
	await position.focus();
	await position.press("ArrowRight");
	await expect
		.poll(async () => Number(await position.inputValue()))
		.toBeGreaterThan(0);
	const thumb = position.locator(
		'xpath=ancestor::*[@data-slot="slider-thumb"]',
	);
	await expect(thumb).toHaveAttribute("data-focus-visible", "true");
	await expect
		.poll(() => thumb.evaluate((el) => getComputedStyle(el).outlineWidth))
		.toBe("2px");

	const volume = page.getByRole("slider", {
		name: "Playback volume",
		exact: true,
	});
	const volumeRoot = volume.locator('xpath=ancestor::*[@data-slot="slider"]');
	const fill = volumeRoot.locator('[data-slot="slider-fill"]');
	// The track's 24px hit area must not become the visible filled bar.
	await expect(volumeRoot.locator('[data-slot="slider-track"]')).toHaveCSS(
		"height",
		"24px",
	);
	await expect(fill).toHaveCSS("height", "6px");
	await volume.focus();
	await volume.press("End");
	await expect(volume).toHaveValue("1");
	await volume.press("ArrowLeft");
	await expect(volume).toHaveValue("0.95");
	await volumeRoot
		.locator("..")
		.screenshot({ path: testInfo.outputPath("volume-slider.png") });

	const handle = page.getByRole("slider", { name: "Clip in point" });
	await handle.focus();
	await expect
		.poll(() => handle.evaluate((el) => getComputedStyle(el).outlineStyle))
		.toBe("solid");
	await expect
		.poll(() => handle.evaluate((el) => getComputedStyle(el, "::after").width))
		.toBe("3px");
	await expect
		.poll(() =>
			handle.evaluate((el) => getComputedStyle(el, "::after").content),
		)
		.toBe('""');

	for (const width of [599, 600, 899, 900]) {
		await page.setViewportSize({ width, height: 1000 });
		await expect(
			page.getByRole("treegrid", { name: "Recordings" }),
		).toHaveCount(width >= 900 ? 1 : 0);
		await expect(
			page.getByRole("button", { name: "Browse files", exact: true }),
		).toHaveCount(width < 900 ? 1 : 0);
		await expect
			.poll(() =>
				page.evaluate(
					() => document.documentElement.scrollWidth <= innerWidth + 1,
				),
			)
			.toBe(true);
	}
	const tree = page.getByRole("treegrid", { name: "Recordings" });
	const year = tree.getByRole("row", { name: "2026", exact: true });
	await year.focus();
	await year.press("ArrowLeft");
	await expect(year).toHaveAttribute("aria-expanded", "false");
	await year.press("ArrowRight");
	await expect(year).toHaveAttribute("aria-expanded", "true");
});

/** A mono 16-bit PCM WAV the browser can actually decode. */
function silentWav(seconds = 1): Buffer {
	const sampleRate = 8_000;
	const dataBytes = sampleRate * 2 * seconds;
	const wav = Buffer.alloc(44 + dataBytes);
	wav.write("RIFF", 0);
	wav.writeUInt32LE(36 + dataBytes, 4);
	wav.write("WAVE", 8);
	wav.write("fmt ", 12);
	wav.writeUInt32LE(16, 16);
	wav.writeUInt16LE(1, 20);
	wav.writeUInt16LE(1, 22);
	wav.writeUInt32LE(sampleRate, 24);
	wav.writeUInt32LE(sampleRate * 2, 28);
	wav.writeUInt16LE(2, 32);
	wav.writeUInt16LE(16, 34);
	wav.write("data", 36);
	wav.writeUInt32LE(dataBytes, 40);
	return wav;
}

interface MockAudioOptions {
	/** Serve a finished silence-free render plus decodable audio bytes. */
	silenceFreeReady?: boolean;
	/**
	 * Length of the served media. Playback assertions need audio that outlasts
	 * the assertion window; a one-second clip can end before it is observed.
	 */
	mediaSeconds?: number;
	/** Serve a ready-to-preview channel mix with one decodable source. */
	channelMixReady?: boolean;
	/** Report realtime as enabled and the session as still recording. */
	liveWithRealtime?: boolean;
}

/** A valid audiowaveform payload with no points, so the preview renders quietly. */
const EMPTY_WAVEFORM_PAYLOAD = Buffer.from(new Uint8Array(20)).toString(
	"base64",
);
const CHANNEL_MIX_SEGMENT_PATH = `/audio/sessions/${SESSION_ID}/segments/1`;
const CHANNEL_MIX_WAVEFORM_PATH =
	"/audio/waveform/guild-123/voice-123/2026/8/channel-mix-source";

declare global {
	interface Window {
		__channelMixProbe: {
			writes: number;
			audios: HTMLAudioElement[];
		};
	}
}

async function mockAudioApi(page: Page, options: MockAudioOptions = {}) {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname.replace(API_PREFIX, "");

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

		if (path === "/users/current") {
			await fulfillJson({
				avatar: "",
				is_dev: false,
				realtime_enabled: options.liveWithRealtime ?? false,
				user_id: "current-user",
				username: "Test Admin",
			});
			return;
		}
		if (path === "/users/current/guilds") {
			await fulfillJson([
				{
					id: GUILD_ID,
					name: "Test Guild",
					owner: true,
					permissions: "8",
				},
			]);
			return;
		}
		if (
			path === `/admin/guilds/${GUILD_ID}/recordings/${SESSION_ID}` &&
			request.method() === "DELETE"
		) {
			await fulfillJson(
				{
					id: "soft-job",
					recording_session_id: SESSION_ID,
					status_url: `/api/admin/guilds/${GUILD_ID}/recording-deletions/soft-job`,
					mode: "soft",
					state: "soft_deleted",
					stage: "soft_deleted",
					attempts: 0,
					error: null,
				},
				202,
			);
			return;
		}
		if (
			path === `/admin/guilds/${GUILD_ID}/recording-deletions/soft-job` &&
			request.method() === "GET"
		) {
			await fulfillJson({
				id: "soft-job",
				recording_session_id: SESSION_ID,
				status_url: `/api/admin/guilds/${GUILD_ID}/recording-deletions/soft-job`,
				mode: "soft",
				state: "soft_deleted",
				stage: "soft_deleted",
				attempts: 0,
				error: null,
			});
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
								8: [
									{
										file: RECORDING_FILE,
										display_name: "Test User",
										recording_session_id: SESSION_ID,
										state: "finalized",
										user_id: "user-123",
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
			await fulfillJson({
				channel_journey: ["voice-123"],
				current_channel_id: "voice-123",
				duration_ms: 30_000,
				ended_at_ms: 1_786_460_430_000,
				events: [
					{
						details: { reason: "test" },
						event_type: "server_mute",
						offset_ms: 4_000,
						source: "voice_state",
					},
					{
						details: {},
						event_type: "server_unmute",
						offset_ms: 10_000,
						source: "voice_state",
					},
				],
				guild_id: GUILD_ID,
				recording_session_id: SESSION_ID,
				segments: [
					{
						audio_file_id: "physical-1",
						channel_id: "voice-123",
						end_ms: 12_000,
						file_name: "physical-1.ogg",
						kind: "file",
						media_url: "/media/physical-1.ogg",
						segment_index: 0,
						start_ms: 0,
					},
					{
						audio_file_id: "physical-2",
						channel_id: "voice-123",
						end_ms: 30_000,
						file_name: "physical-2.ogg",
						kind: "file",
						media_url: "/media/physical-2.ogg",
						segment_index: 1,
						start_ms: 12_000,
					},
				],
				started_at_ms: 1_786_460_400_000,
				starting_channel_id: "voice-123",
				state: options.liveWithRealtime ? "active" : "finalized",
				user_id: "user-123",
			});
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/waveform`) {
			await fulfillJson({ building: false, progress: 100 });
			return;
		}
		if (
			options.silenceFreeReady &&
			path === `/audio/sessions/${SESSION_ID}/silence-free/waveform`
		) {
			await fulfillJson({ building: false, progress: 100 });
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/channel-mix`) {
			if (!options.channelMixReady) {
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
			await fulfillJson({
				can_generate: true,
				duration_ms: 30_000,
				generation_settings: null,
				media_url: null,
				participants: [
					{
						display_name: "Test User",
						session_ids: [SESSION_ID],
						source_count: 1,
						user_id: "user-123",
					},
				],
				progress: 0,
				reason: null,
				scope: "all_recordings",
				source_count: 1,
				status: "idle",
				tracks: [
					{
						display_name: "Test User",
						is_anchor: true,
						segments: [
							{
								audio_file_id: "1",
								end_ms: 30_000,
								hls_playlist_url: `/api/audio/sessions/${SESSION_ID}/live/1/playlist.m3u8`,
								id: "1:0",
								live: false,
								media_url: `/api/audio/sessions/${SESSION_ID}/segments/1`,
								recording_session_id: SESSION_ID,
								source_duration_ms: 30_000,
								source_offset_ms: 0,
								start_ms: 0,
								waveform_url: `/api${CHANNEL_MIX_WAVEFORM_PATH}`,
							},
						],
						user_id: "user-123",
					},
				],
			});
			return;
		}
		if (options.channelMixReady && path === CHANNEL_MIX_SEGMENT_PATH) {
			await route.fulfill({
				status: 200,
				headers: {
					...corsHeaders,
					"Accept-Ranges": "none",
					"Content-Type": "audio/wav",
				},
				body: silentWav(options.mediaSeconds ?? 30),
			});
			return;
		}
		if (options.channelMixReady && path === CHANNEL_MIX_WAVEFORM_PATH) {
			await fulfillJson({ data: EMPTY_WAVEFORM_PAYLOAD, progress: 100 });
			return;
		}
		if (path === `/audio/sessions/${SESSION_ID}/remove-silence`) {
			await fulfillJson(
				options.silenceFreeReady
					? { status: "ready", progress: 100 }
					: { status: "idle", progress: 0 },
			);
			return;
		}
		if (
			options.silenceFreeReady &&
			path === `/audio/sessions/${SESSION_ID}/silence-free`
		) {
			await route.fulfill({
				status: 200,
				headers: {
					...corsHeaders,
					"Accept-Ranges": "none",
					"Content-Type": "audio/wav",
				},
				body: silentWav(options.mediaSeconds),
			});
			return;
		}
		if (options.silenceFreeReady && path.startsWith("/media/")) {
			await route.fulfill({
				status: 200,
				headers: {
					...corsHeaders,
					"Accept-Ranges": "none",
					"Content-Type": "audio/wav",
				},
				body: silentWav(options.mediaSeconds),
			});
			return;
		}

		await fulfillJson(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
}

test("a short multi-file session keeps its draft inside the clip window", async ({
	page,
}) => {
	const updateDepthErrors: string[] = [];
	const styleWarnings: string[] = [];
	page.on("console", (message) => {
		if (message.text().includes("Maximum update depth exceeded")) {
			updateDepthErrors.push(message.text());
		}
		if (message.text().includes("Updating a style property during rerender")) {
			styleWarnings.push(message.text());
		}
	});
	page.on("pageerror", (error) => {
		if (error.message.includes("Maximum update depth exceeded")) {
			updateDepthErrors.push(error.message);
		}
		if (error.message.includes("Updating a style property during rerender")) {
			styleWarnings.push(error.message);
		}
	});
	await mockAudioApi(page);

	await page.goto(`/dashboard/${GUILD_ID}/audio`);
	const isMobile = (page.viewportSize()?.width ?? 1_000) < 900;
	if (isMobile) {
		const browseButton = page.getByRole("button", { name: "Browse files" });
		await expect(browseButton).toBeVisible();
		const [headerBounds, browseBounds] = await Promise.all([
			page.locator("header").first().boundingBox(),
			browseButton.boundingBox(),
		]);
		expect(headerBounds).not.toBeNull();
		expect(browseBounds).not.toBeNull();
		if (headerBounds && browseBounds) {
			expect(
				browseBounds.y - (headerBounds.y + headerBounds.height),
			).toBeLessThan(20);
		}
		await expect(
			page.getByRole("treegrid", { name: "Recordings" }),
		).toHaveCount(0);
		await expect(
			page.getByRole("button", { name: "open navigation" }),
		).toHaveCount(0);
		await expect(
			page.getByRole("button", { name: "Audio", exact: true }),
		).toBeVisible();
		await browseButton.click();
		await expect(
			page.getByRole("treegrid", { name: "Recordings" }),
		).toBeVisible();
		await expect
			.poll(() =>
				page.evaluate(
					() => document.documentElement.scrollWidth <= window.innerWidth + 1,
				),
			)
			.toBe(true);
	} else {
		await expect(
			page.getByRole("treegrid", { name: "Recordings" }),
		).toBeVisible();
	}
	await expect(
		page.getByRole("textbox", { name: "Search recordings" }),
	).toBeVisible();
	await page.getByRole("button", { name: "Collapse 2026" }).click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeHidden();
	await page.getByRole("button", { name: "Expand 2026" }).click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeVisible();
	// Pressing anywhere on a parent row must toggle it, not just the chevron.
	const yearRow = page.locator('[role="row"][data-key="2026"]');
	await yearRow.click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeHidden();
	await yearRow.click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeVisible();
	const monthRow = page.locator('[role="row"][data-key="2026-8"]');
	await monthRow.click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeHidden();
	await monthRow.click();
	await expect(page.getByTitle(RECORDING_FILE)).toBeVisible();
	await page.getByTitle(RECORDING_FILE).click();
	if (isMobile) {
		await expect(
			page.getByRole("treegrid", { name: "Recordings" }),
		).toHaveCount(0);
	}

	await expect(page).toHaveURL(
		new RegExp(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}$`),
	);
	if (!isMobile) {
		// With a recording selected the press routes through selection instead of
		// the primary action; it must still toggle rather than navigate away.
		await yearRow.click();
		await expect(page.getByTitle(RECORDING_FILE)).toBeHidden();
		await yearRow.click();
		await expect(page.getByTitle(RECORDING_FILE)).toBeVisible();
		await expect(page).toHaveURL(
			new RegExp(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}$`),
		);
	}
	await expect(
		page.getByText(`Session ${SESSION_ID}`, { exact: true }),
	).toBeVisible();
	const positionSlider = page.getByRole("slider", {
		name: "Logical playback position",
	});
	await expect(positionSlider).toBeVisible();
	await expect
		.poll(() =>
			positionSlider
				.locator('xpath=ancestor::*[@data-slot="slider-thumb"]')
				.evaluate((element) => getComputedStyle(element).backgroundColor),
		)
		.toContain("144, 202, 249");
	const playButton = page.getByRole("button", { name: "Play" });
	await expect
		.poll(() =>
			playButton.evaluate(
				(element) => getComputedStyle(element).backgroundColor,
			),
		)
		.toBe("rgb(144, 202, 249)");
	const playbackVolumeSlider = page.getByRole("slider", {
		name: "Playback volume",
	});
	await expect
		.poll(() =>
			playbackVolumeSlider
				.locator('xpath=ancestor::*[@data-slot="slider-thumb"]')
				.evaluate((element) => getComputedStyle(element).backgroundColor),
		)
		.toContain("144, 202, 249");
	await positionSlider.scrollIntoViewIfNeeded();
	const positionBounds = await positionSlider
		.locator('xpath=ancestor::*[@data-slot="slider-track"]')
		.boundingBox();
	expect(positionBounds).not.toBeNull();
	if (positionBounds) {
		const y = positionBounds.y + positionBounds.height / 2;
		await page.mouse.move(positionBounds.x + positionBounds.width * 0.05, y);
		await page.mouse.down();
		await page.mouse.move(positionBounds.x + positionBounds.width * 0.35, y);
		await page.mouse.up();
	}
	await expect
		.poll(async () => Number(await positionSlider.inputValue()))
		.toBeGreaterThan(0);
	await expect.poll(() => updateDepthErrors).toEqual([]);
	if (isMobile) {
		const speedSlider = page.getByRole("slider", {
			name: "Playback speed",
		});
		const [volumeBounds, speedBounds] = await Promise.all([
			playbackVolumeSlider
				.locator('xpath=ancestor::*[@data-slot="slider"]')
				.boundingBox(),
			speedSlider
				.locator('xpath=ancestor::*[@data-slot="slider"]')
				.boundingBox(),
		]);
		expect(volumeBounds).not.toBeNull();
		expect(speedBounds).not.toBeNull();
		if (volumeBounds && speedBounds) {
			expect(Math.abs(volumeBounds.y - speedBounds.y)).toBeLessThan(1);
		}

		const downloadSession = page.getByRole("button", {
			name: "Download session",
		});
		const removeSilence = page.getByRole("button", {
			name: "Remove silence",
		});
		const [downloadBounds, removeBounds] = await Promise.all([
			downloadSession.boundingBox(),
			removeSilence.boundingBox(),
		]);
		expect(downloadBounds).not.toBeNull();
		expect(removeBounds).not.toBeNull();
		if (downloadBounds && removeBounds) {
			expect(Math.abs(downloadBounds.y - removeBounds.y)).toBeLessThan(1);
		}
	}
	const eventTimeline = page.getByRole("button", { name: /Event timeline/ });
	await eventTimeline.click();
	// Tailwind v4 resolves opacity-modified tokens through color-mix(), which
	// Chrome serializes in oklab rather than rgba. Compare against a probe
	// carrying the same utility so this asserts the colour, not its spelling.
	const mutedSurface = await page.evaluate(() => {
		const probe = document.createElement("div");
		probe.className = "bg-muted/4";
		document.body.append(probe);
		const background = getComputedStyle(probe).backgroundColor;
		probe.remove();
		return background;
	});
	await expect
		.poll(() =>
			eventTimeline.evaluate(
				(element) => getComputedStyle(element).backgroundColor,
			),
		)
		.toBe(mutedSurface);
	await expect
		.poll(() =>
			eventTimeline.evaluate((element) => getComputedStyle(element).color),
		)
		.toBe("rgb(148, 163, 184)");
	const mutedInterval = page.getByRole("button", {
		name: "Server muted, 00:00:04 to 00:00:10",
	});
	await expect(mutedInterval).toBeVisible();
	await mutedInterval.hover();
	await expect(page.getByRole("tooltip")).toContainText("Server muted");
	const inPoint = page.getByRole("slider", { name: "Clip in point" });
	const outPoint = page.getByRole("slider", { name: "Clip out point" });
	await expect(inPoint).toHaveAttribute("aria-valuenow", "0");
	await expect(outPoint).toHaveAttribute("aria-valuenow", "15000");
	for (const handle of [inPoint, outPoint]) {
		expect(
			await handle.evaluate(
				(element) => getComputedStyle(element).backgroundColor,
			),
		).not.toBe("rgba(0, 0, 0, 0)");
	}

	const actionThumbs = page.getByRole("slider", {
		name: "Logical action range",
	});
	await expect(actionThumbs).toHaveCount(2);
	const firstThumb = actionThumbs.nth(0);
	await expect
		.poll(() =>
			firstThumb
				.locator('xpath=ancestor::*[@data-slot="slider-thumb"]')
				.evaluate((element) => getComputedStyle(element).backgroundColor),
		)
		.toBe("rgb(144, 202, 249)");
	await firstThumb.scrollIntoViewIfNeeded();
	const sliderRoot = firstThumb.locator(
		'xpath=ancestor::*[@data-slot="slider-track"]',
	);
	const thumbBounds = await firstThumb
		.locator('xpath=ancestor::*[@data-slot="slider-thumb"]')
		.boundingBox();
	const sliderBounds = await sliderRoot.boundingBox();
	if (thumbBounds && sliderBounds) {
		await page.mouse.move(
			thumbBounds.x + thumbBounds.width / 2,
			thumbBounds.y + thumbBounds.height / 2,
		);
		await page.mouse.down();
		await page.mouse.move(
			sliderBounds.x + sliderBounds.width * 0.2,
			thumbBounds.y + thumbBounds.height / 2,
		);
		await page.mouse.up();
	}
	await expect(firstThumb).toHaveValue("6000");
	await page.keyboard.press("r");
	await expect(inPoint).toHaveAttribute("aria-valuenow", "0");

	const clipNameInput = page.getByLabel("Clip name");
	const createClipButton = page.getByRole("button", { name: "Create clip" });
	const [clipNameBounds, createClipBounds] = await Promise.all([
		clipNameInput.boundingBox(),
		createClipButton.boundingBox(),
	]);
	expect(clipNameBounds?.height).toBe(createClipBounds?.height);
	expect(clipNameBounds?.height).toBe(40);
	if (clipNameBounds && createClipBounds) {
		expect(Math.abs(clipNameBounds.y - createClipBounds.y)).toBeLessThan(1);
	}

	const sessionWindow = page.getByTestId("clip-session-window");
	await sessionWindow.scrollIntoViewIfNeeded();
	const bounds = await sessionWindow.boundingBox();
	expect(bounds).not.toBeNull();
	if (bounds) {
		await page.mouse.move(
			bounds.x + bounds.width * 0.25,
			bounds.y + bounds.height / 2,
		);
		await page.mouse.down();
		await page.mouse.move(
			bounds.x + bounds.width * 0.45,
			bounds.y + bounds.height / 2,
		);
		await page.mouse.up();
	}
	await expect
		.poll(async () => {
			const start = Number(await inPoint.getAttribute("aria-valuenow"));
			const end = Number(await outPoint.getAttribute("aria-valuenow"));
			return end - start;
		})
		.toBe(15_000);
	await expect.poll(() => updateDepthErrors).toEqual([]);
	await expect.poll(() => styleWarnings).toEqual([]);

	await page.reload();
	await expect(
		page.getByText(`Session ${SESSION_ID}`, { exact: true }),
	).toBeVisible();
	await expect(inPoint).toHaveAttribute("aria-valuenow", "0");
	await expect(outPoint).toHaveAttribute("aria-valuenow", "15000");

	await page.goto(
		`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}?t=20&clip=stamp`,
	);
	await expect(inPoint).toHaveAttribute("aria-valuenow", "10000");
	await expect(outPoint).toHaveAttribute("aria-valuenow", "25000");
	await expect.poll(() => updateDepthErrors).toEqual([]);
});

test("playback shortcuts follow the visible tab after switching", async ({
	page,
}) => {
	await mockAudioApi(page, { silenceFreeReady: true, mediaSeconds: 30 });
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);

	const normalPanel = page.getByRole("tabpanel", {
		name: "Normal",
		exact: true,
	});
	await expect(normalPanel).toBeVisible();
	await normalPanel.getByRole("button", { name: "Play", exact: true }).click();
	await expect(
		normalPanel.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();

	await page.getByRole("tab", { name: "Silence-free", exact: true }).click();
	const silencePanel = page.getByRole("tabpanel", {
		name: "Silence-free",
		exact: true,
	});
	await expect(silencePanel).toBeVisible();

	// Focus stays on the tab, which owns Space in react-aria; a user pressing
	// Space from the page body must drive the player they can see, not the
	// paused Normal player that is now hidden.
	await page.evaluate(() => {
		(document.activeElement as HTMLElement | null)?.blur();
	});
	await page.keyboard.press("Space");

	await expect(
		silencePanel.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible({ timeout: 10_000 });
});

test("channel mix playback starts once and never re-seeks from canplay", async ({
	page,
}) => {
	await page.addInitScript(() => {
		const probe: Window["__channelMixProbe"] = { audios: [], writes: 0 };
		window.__channelMixProbe = probe;
		// A construct-trap proxy keeps `new Audio()` semantics while collecting
		// the elements the channel mix preview builds.
		window.Audio = new Proxy(window.Audio, {
			construct(target, args) {
				const element = Reflect.construct(target, args) as HTMLAudioElement;
				probe.audios.push(element);
				return element;
			},
		});
		const descriptor = Object.getOwnPropertyDescriptor(
			HTMLMediaElement.prototype,
			"currentTime",
		);
		Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
			configurable: true,
			get: descriptor?.get,
			set(value: number) {
				probe.writes += 1;
				descriptor?.set?.call(this, value);
			},
		});
	});
	await mockAudioApi(page, { channelMixReady: true, mediaSeconds: 30 });
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);

	const mixTab = page.getByRole("tab", { name: "Channel mix", exact: true });
	await expect(mixTab).toBeVisible();
	await mixTab.click();
	const mixPanel = page.getByRole("tabpanel", {
		name: "Channel mix",
		exact: true,
	});
	await expect(mixPanel).toBeVisible();
	await mixPanel.getByRole("button", { name: "Play", exact: true }).click();
	await expect(
		mixPanel.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();
	const isPlaying = () =>
		page.evaluate(() =>
			window.__channelMixProbe.audios.some((element) => !element.paused),
		);
	await expect.poll(isPlaying).toBe(true);

	// A `canplay` event on an already-playing source must not reposition it.
	// Dispatching synchronously keeps the measurement free of the unrelated
	// animation-frame drift correction, so any write counted here comes from
	// the `canplay` handler. Seeking there re-fires `canplay`, so the old code
	// looped at ~500 seeks a second and every mix source stuttered.
	const canplayWrites = await page.evaluate(() => {
		const probe = window.__channelMixProbe;
		const playing = probe.audios.find((element) => !element.paused);
		if (!playing) return -1;
		const before = probe.writes;
		for (let index = 0; index < 20; index += 1) {
			playing.dispatchEvent(new Event("canplay"));
		}
		return probe.writes - before;
	});
	expect(canplayWrites).toBe(0);

	// Steady playback stays released; the loop produced hundreds of writes per
	// second, so a small bound still separates it from legitimate corrections.
	const writesBefore = await page.evaluate(
		() => window.__channelMixProbe.writes,
	);
	await page.waitForTimeout(1_500);
	const writesAfter = await page.evaluate(
		() => window.__channelMixProbe.writes,
	);
	expect(writesAfter - writesBefore).toBeLessThanOrEqual(10);
	await expect.poll(isPlaying).toBe(true);
});

test("the playhead glides over audio instead of stepping on timeupdate", async ({
	page,
}) => {
	await mockAudioApi(page, { silenceFreeReady: true, mediaSeconds: 30 });
	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);

	const normalPanel = page.getByRole("tabpanel", {
		name: "Normal",
		exact: true,
	});
	await expect(normalPanel).toBeVisible();
	await normalPanel.getByRole("button", { name: "Play", exact: true }).click();
	await expect(
		normalPanel.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();
	const positionSlider = page.getByRole("slider", {
		name: "Logical playback position",
	});
	await expect
		.poll(async () => Number(await positionSlider.inputValue()))
		.toBeGreaterThan(0);

	// Sample the playhead every animation frame. `timeupdate` alone advances it
	// only a few times a second, which is what made the head step over audio
	// while the (per-frame) silence branches glided.
	const distinctPositions = await positionSlider.evaluate((element) => {
		const input = element as HTMLInputElement;
		const seen = new Set<string>();
		const deadline = performance.now() + 600;
		return new Promise<number>((resolve) => {
			const sample = () => {
				seen.add(input.value);
				if (performance.now() >= deadline) {
					resolve(seen.size);
					return;
				}
				requestAnimationFrame(sample);
			};
			requestAnimationFrame(sample);
		});
	});
	expect(distinctPositions).toBeGreaterThan(10);
	await expect(
		normalPanel.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();
});

test("a live session keeps loading new audio while realtime is connected", async ({
	page,
}) => {
	// A growing recording changes only its heartbeat, which sends no realtime
	// event, so the manifest poll must run even while the socket is live.
	await mockAudioApi(page, { liveWithRealtime: true });
	let subscribed = false;
	await page.routeWebSocket(
		`${API_ORIGIN}/api/realtime`.replace("http", "ws"),
		(ws) => {
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
				subscribed = true;
			});
		},
	);
	let manifestLoads = 0;
	page.on("request", (request) => {
		if (
			new URL(request.url()).pathname ===
			`${API_PREFIX}/audio/sessions/${SESSION_ID}/manifest`
		)
			manifestLoads += 1;
	});

	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	await expect.poll(() => subscribed).toBe(true);
	await expect.poll(() => manifestLoads).toBeGreaterThanOrEqual(1);
	const before = manifestLoads;
	// The poll runs every 5 s.
	await expect
		.poll(() => manifestLoads, { timeout: 8_000 })
		.toBeGreaterThan(before);
});
