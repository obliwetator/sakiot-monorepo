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
				events: [],
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
	page.on("console", (message) => {
		if (message.text().includes("Maximum update depth exceeded")) {
			updateDepthErrors.push(message.text());
		}
	});
	page.on("pageerror", (error) => {
		if (error.message.includes("Maximum update depth exceeded")) {
			updateDepthErrors.push(error.message);
		}
	});
	await mockAudioApi(page);

	await page.goto(`/dashboard/${GUILD_ID}/audio`);
	const isMobile = (page.viewportSize()?.width ?? 1_000) < 900;
	if (isMobile) {
		const browseButton = page.getByRole("button", { name: "Browse files" });
		await expect(browseButton).toBeVisible();
		await browseButton.click();
	}
	await page.getByTitle(RECORDING_FILE).click();

	await expect(page).toHaveURL(
		new RegExp(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}$`),
	);
	if (isMobile) await page.keyboard.press("Escape");
	await expect(
		page.getByText(`Session ${SESSION_ID}`, { exact: true }),
	).toBeVisible();
	const inPoint = page.getByRole("slider", { name: "Clip in point" });
	const outPoint = page.getByRole("slider", { name: "Clip out point" });
	await expect(inPoint).toHaveAttribute("aria-valuenow", "0");
	await expect(outPoint).toHaveAttribute("aria-valuenow", "15000");
	const clipNameInput = page.getByLabel("Clip name");
	const createClipButton = page.getByRole("button", { name: "Create clip" });
	const [clipNameBounds, createClipBounds] = await Promise.all([
		clipNameInput.boundingBox(),
		createClipButton.boundingBox(),
	]);
	expect(clipNameBounds?.height).toBe(createClipBounds?.height);
	expect(clipNameBounds?.height).toBe(40);

	const sessionWindow = page.getByTestId("clip-session-window");
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
