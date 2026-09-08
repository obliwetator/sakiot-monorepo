import { expect, test } from "@playwright/test";
import { API_ORIGIN, corsHeaders, GUILD_ID } from "./clip-editor-fixture";

const API_PREFIX = "/api";
const CLIP_COUNT = 40;

/** Long list so the selection pane must scroll instead of growing the page. */
async function mockManyClips(page: import("@playwright/test").Page) {
	const clips = Array.from({ length: CLIP_COUNT }, (_, index) => ({
		channel_id: "channel-123",
		clip_id: `clip-${index}`,
		guild_id: GUILD_ID,
		length: 3,
		name: `Clip number ${index}`,
		original_file_name: `clip-${index}.wav`,
		start_time: 0,
		user_id: "user-123",
	}));

	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname.replace(API_PREFIX, "");
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: corsHeaders });
			return;
		}
		const fulfillJson = async (body: unknown) => {
			await route.fulfill({
				status: 200,
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
				{ id: GUILD_ID, name: "Test Guild", owner: true, permissions: "8" },
			]);
			return;
		}
		if (path === `/audio/clips/${GUILD_ID}`) {
			await fulfillJson(clips);
			return;
		}
		await route.fulfill({ status: 204, headers: corsHeaders });
	});
}

test("clip selection is confined to the viewport and scrolls", async ({
	page,
	isMobile,
}) => {
	await mockManyClips(page);
	await page.goto(`/dashboard/${GUILD_ID}/clips`);
	if (isMobile) {
		await page.getByRole("button", { name: "Browse clips" }).click();
	}

	const firstClip = page.getByRole("button", { name: /Clip number 0\b/ });
	await expect(firstClip).toBeVisible();

	// Walk up from a clip row to whichever ancestor actually scrolls.
	const metrics = await firstClip.evaluate((element) => {
		let node: HTMLElement | null = element.parentElement;
		while (node) {
			const style = getComputedStyle(node);
			if (
				/(auto|scroll)/.test(style.overflowY) &&
				node.scrollHeight > node.clientHeight
			) {
				const rect = node.getBoundingClientRect();
				return {
					found: true,
					clientHeight: node.clientHeight,
					scrollHeight: node.scrollHeight,
					top: rect.top,
					bottom: rect.bottom,
					overflowY: style.overflowY,
					viewport: window.innerHeight,
					docScrollHeight: document.documentElement.scrollHeight,
				};
			}
			node = node.parentElement;
		}
		return {
			found: false,
			clientHeight: 0,
			scrollHeight: 0,
			top: 0,
			bottom: 0,
			overflowY: "none",
			viewport: window.innerHeight,
			docScrollHeight: document.documentElement.scrollHeight,
		};
	});

	expect(metrics.found).toBe(true);
	// Fits on screen, scrolls internally, and does not stretch the page.
	expect(metrics.top).toBeGreaterThanOrEqual(0);
	expect(metrics.bottom).toBeLessThanOrEqual(metrics.viewport + 1);
	expect(metrics.scrollHeight).toBeGreaterThan(metrics.clientHeight);
	expect(metrics.overflowY).toBe("auto");
	expect(metrics.docScrollHeight).toBeLessThanOrEqual(metrics.viewport + 1);

	// Capture the pinned controls before anything scrolls, so the comparison
	// below is against the top of the list.
	const editorButton = page.getByRole("button", {
		name: "Clip editor",
		exact: true,
	});
	const clipsTab = page.getByRole("tab", { name: "Clips" });
	const editorBefore = await editorButton.boundingBox();
	const tabBefore = await clipsTab.boundingBox();
	expect(editorBefore).not.toBeNull();
	expect(tabBefore).not.toBeNull();

	if (!isMobile) {
		// The player pane must stay put while the list scrolls. Scrolling the
		// whole page instead is exactly what this guards against.
		await firstClip.click();
		await expect(page).toHaveURL(/\/clips\/clip-0$/);
		const player = page.getByRole("slider", { name: "Clip playback position" });
		await expect(player).toBeVisible();
		const before = await player.boundingBox();

		const lastClip = page.getByRole("button", { name: /Clip number 39\b/ });
		await lastClip.scrollIntoViewIfNeeded();
		await expect(lastClip).toBeVisible();

		const after = await player.boundingBox();
		expect(before).not.toBeNull();
		expect(after).not.toBeNull();
		if (before && after) expect(Math.abs(after.y - before.y)).toBeLessThan(2);
	}

	// The editor link and the tab strip are pinned: only the rows scroll.
	await page
		.getByRole("button", { name: /Clip number 39\b/ })
		.scrollIntoViewIfNeeded();

	await expect(editorButton).toBeVisible();
	await expect(clipsTab).toBeVisible();
	const editorAfter = await editorButton.boundingBox();
	const tabAfter = await clipsTab.boundingBox();
	expect(editorAfter).not.toBeNull();
	expect(tabAfter).not.toBeNull();
	if (editorBefore && editorAfter) {
		expect(Math.abs(editorAfter.y - editorBefore.y)).toBeLessThan(2);
	}
	if (tabBefore && tabAfter) {
		expect(Math.abs(tabAfter.y - tabBefore.y)).toBeLessThan(2);
	}
});
