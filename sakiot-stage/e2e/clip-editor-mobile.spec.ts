import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, POST, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

const clips = [
	{
		channel_id: "channel-123",
		clip_id: "working-source",
		guild_id: GUILD_ID,
		length: 3,
		name: "Working source",
		original_file_name: "working-source.wav",
		silence_free: false,
		start_time: 0,
		user_id: "user-123",
	},
	{
		channel_id: "channel-123",
		clip_id: "broken-source",
		guild_id: GUILD_ID,
		length: 2,
		name: "Broken source",
		original_file_name: "broken-source.wav",
		silence_free: false,
		start_time: 0,
		user_id: "user-123",
	},
];

function silentWav(): Buffer {
	const frames = 800;
	const sampleRate = 8_000;
	const dataBytes = frames * 2;
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

async function mockClipEditorApi(page: Page) {
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
				username: "Mobile Editor User",
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
		if (path === `/audio/clips/${GUILD_ID}`) {
			await fulfillJson(clips);
			return;
		}
		if (path === `/audio/clips/${GUILD_ID}/working-source`) {
			await route.fulfill({
				status: 200,
				headers: { ...corsHeaders, "Content-Type": "audio/wav" },
				body: silentWav(),
			});
			return;
		}
		if (path === `/audio/clips/${GUILD_ID}/broken-source`) {
			await new Promise((resolve) => setTimeout(resolve, 300));
			await fulfillJson({ detail: "Clip audio is unavailable" }, 500);
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
}

test.beforeEach(async ({ page }, testInfo) => {
	test.skip(
		testInfo.project.name !== "mobile-chromium",
		"This regression is specific to the compact clip-editor layout.",
	);
	await page.setViewportSize({ width: 375, height: 667 });
	await mockClipEditorApi(page);
});

test("the source drawer gives the timeline room and recovers rejected additions", async ({
	page,
}) => {
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);

	const browse = page.getByRole("button", { name: "Browse files" });
	const timeline = page.getByLabel("Clip editor timeline");
	const inspector = page.getByTestId("clip-inspector");
	const search = page.getByPlaceholder("Search clips");

	await expect(browse).toBeVisible();
	await expect(search).toBeHidden();
	await expect(timeline).toBeVisible();
	await expect(inspector).toContainText("No segment selected.");

	const emptyTimelineBounds = await timeline.boundingBox();
	expect(emptyTimelineBounds?.height).toBeGreaterThan(120);

	await browse.click();
	const workingSource = page.getByRole("button", { name: /Working source/ });
	await expect(search).toBeVisible();
	await expect(workingSource).toBeVisible();
	const workingSourceElement = await workingSource.elementHandle();
	expect(workingSourceElement).not.toBeNull();

	// Starting a hold/drag gets the source drawer out of the way. Ending the
	// drag without a timeline drop restores it so the user does not get stuck.
	const rejectedTransfer = await page.evaluateHandle(() => new DataTransfer());
	await workingSourceElement?.dispatchEvent("dragstart", {
		dataTransfer: rejectedTransfer,
	});
	await expect(search).toBeHidden();
	await workingSourceElement?.dispatchEvent("dragend", {
		dataTransfer: rejectedTransfer,
	});
	await expect(search).toBeVisible();

	// DevTools and real touch devices do not emit native HTML drag events. A
	// long press therefore switches to the explicit touch-drag path, which
	// leaves the drawer closed after a successful timeline drop.
	const dropzone = page.getByTestId("clip-timeline-dropzone");
	const dropBounds = await dropzone.boundingBox();
	const sourceBounds = await workingSource.boundingBox();
	expect(dropBounds).not.toBeNull();
	expect(sourceBounds).not.toBeNull();
	const touchStart = {
		x: (sourceBounds?.x ?? 0) + (sourceBounds?.width ?? 1) / 2,
		y: (sourceBounds?.y ?? 0) + (sourceBounds?.height ?? 1) / 2,
	};
	const touchEnd = {
		x: (dropBounds?.x ?? 0) + (dropBounds?.width ?? 1) / 2,
		y: (dropBounds?.y ?? 0) + 50,
	};
	await workingSource.evaluate((element, point) => {
		const touch = new Touch({
			identifier: 17,
			target: element,
			clientX: point.x,
			clientY: point.y,
		});
		element.dispatchEvent(
			new TouchEvent("touchstart", {
				bubbles: true,
				cancelable: true,
				touches: [touch],
				targetTouches: [touch],
				changedTouches: [touch],
			}),
		);
	}, touchStart);
	await page.waitForTimeout(350);
	await expect
		.poll(() =>
			page
				.getByTestId("clip-source-bin")
				.evaluate(
					(element) =>
						getComputedStyle(element.parentElement as HTMLElement).opacity,
				),
		)
		.toBe("0");
	await workingSource.evaluate((element, point) => {
		const touch = new Touch({
			identifier: 17,
			target: element,
			clientX: point.x,
			clientY: point.y,
		});
		window.dispatchEvent(
			new TouchEvent("touchmove", {
				bubbles: true,
				cancelable: true,
				touches: [touch],
				targetTouches: [touch],
				changedTouches: [touch],
			}),
		);
	}, touchEnd);
	await expect(page.getByTestId("clip-drag-ghost")).toBeVisible();
	await workingSource.evaluate((element, point) => {
		const touch = new Touch({
			identifier: 17,
			target: element,
			clientX: point.x,
			clientY: point.y,
		});
		window.dispatchEvent(
			new TouchEvent("touchend", {
				bubbles: true,
				cancelable: true,
				touches: [],
				targetTouches: [],
				changedTouches: [touch],
			}),
		);
	}, touchEnd);
	await expect(timeline.getByText("Working source")).toBeVisible();
	await expect(search).toBeHidden();

	await timeline.getByText("Working source").click();
	await expect(inspector).toContainText("Working source");
	const layout = await inspector.evaluate((element) => {
		const inspectorBounds = element.getBoundingClientRect();
		const parentBounds = element.parentElement?.getBoundingClientRect();
		const timelineBounds = document
			.querySelector('[aria-label="Clip editor timeline"]')
			?.getBoundingClientRect();
		return {
			inspectorHeight: inspectorBounds.height,
			inspectorTop: inspectorBounds.top,
			parentHeight: parentBounds?.height ?? 1,
			timelineBottom: timelineBounds?.bottom ?? 0,
		};
	});
	expect(layout.inspectorTop).toBeGreaterThanOrEqual(layout.timelineBottom - 1);
	expect(layout.inspectorHeight / layout.parentHeight).toBeGreaterThan(0.3);
	expect(layout.inspectorHeight / layout.parentHeight).toBeLessThan(0.35);

	// Tapping is the touch-browser fallback. A decode failure restores the
	// drawer after briefly revealing the timeline during the attempt.
	await browse.click();
	const brokenSource = page.getByRole("button", { name: /Broken source/ });
	await brokenSource.click();
	await expect(search).toBeHidden();
	await expect(search).toBeVisible();
});

test("a wide touch viewport does not enter a stuck native drag", async ({
	page,
}) => {
	await page.setViewportSize({ width: 1000, height: 700 });
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);

	const search = page.getByPlaceholder("Search clips");
	const workingSource = page.getByRole("button", { name: /Working source/ });
	await expect(page.getByRole("button", { name: "Browse files" })).toBeHidden();
	await expect(search).toBeVisible();
	await expect(workingSource).toBeVisible();
	await expect(workingSource).toHaveAttribute("draggable", "false");

	const sourceBounds = await workingSource.boundingBox();
	expect(sourceBounds).not.toBeNull();
	const point = {
		x: (sourceBounds?.x ?? 0) + (sourceBounds?.width ?? 1) / 2,
		y: (sourceBounds?.y ?? 0) + (sourceBounds?.height ?? 1) / 2,
	};
	await workingSource.evaluate((element, position) => {
		const touch = new Touch({
			identifier: 23,
			target: element,
			clientX: position.x,
			clientY: position.y,
		});
		element.dispatchEvent(
			new TouchEvent("touchstart", {
				bubbles: true,
				cancelable: true,
				touches: [touch],
				targetTouches: [touch],
				changedTouches: [touch],
			}),
		);
	}, point);
	await page.waitForTimeout(350);
	await workingSource.evaluate((element, position) => {
		const touch = new Touch({
			identifier: 23,
			target: element,
			clientX: position.x,
			clientY: position.y,
		});
		window.dispatchEvent(
			new TouchEvent("touchend", {
				bubbles: true,
				cancelable: true,
				touches: [],
				targetTouches: [],
				changedTouches: [touch],
			}),
		);
	}, point);

	await search.fill("Broken");
	await expect(
		page.getByRole("button", { name: /Broken source/ }),
	).toBeVisible();
});
