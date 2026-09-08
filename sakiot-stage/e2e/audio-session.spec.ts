import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";
const SESSION_ID = "session-123";
const RECORDING_FILE = "1786460400000-Test_User.ogg";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

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

async function mockAudioApi(page: Page) {
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname.replace(API_PREFIX, "");

		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}

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
				state: "finalized",
				user_id: "user-123",
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
