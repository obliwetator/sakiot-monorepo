import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";
const CHANNEL_ID = "voice-123";
const FILE_NAME = "1786460400000-Test_User";

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token, Range",
	"Access-Control-Allow-Methods": "GET, HEAD, POST, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
	"Access-Control-Expose-Headers": "Content-Range, Accept-Ranges",
};

/** A mono 16-bit PCM WAV the browser can actually decode. */
function silentWav(durationSeconds = 1): Buffer {
	const sampleRate = 8_000;
	const frames = sampleRate * durationSeconds;
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

async function mockRecordingApi(page: Page, durationSeconds = 1) {
	const audioPath = `${API_PREFIX}/audio/${GUILD_ID}/${CHANNEL_ID}/2026/8/${FILE_NAME}.ogg`;
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname;

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

		if (path === `${API_PREFIX}/users/current`) {
			await fulfillJson({
				avatar: "",
				is_dev: false,
				user_id: "current-user",
				username: "Test Admin",
			});
			return;
		}
		if (path === `${API_PREFIX}/users/current/guilds`) {
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
		if (path === `${API_PREFIX}/current/${GUILD_ID}`) {
			await fulfillJson([
				{
					channel_id: CHANNEL_ID,
					dirs: [
						{
							year: 2026,
							months: {
								8: [
									{
										file: `${FILE_NAME}.ogg`,
										display_name: "Test User",
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
		if (path === `${API_PREFIX}/current/${GUILD_ID}/live-stems`) {
			await fulfillJson([]);
			return;
		}
		if (path.endsWith(`/${FILE_NAME}/state`)) {
			await fulfillJson({
				ended_at: 1_786_460_430_000,
				live: false,
				started_at: 1_786_460_400_000,
			});
			return;
		}
		if (path === audioPath) {
			if (request.method() === "HEAD") {
				await route.fulfill({ status: 404, headers: corsHeaders });
				return;
			}
			await route.fulfill({
				status: 200,
				headers: {
					...corsHeaders,
					"Accept-Ranges": "none",
					"Content-Type": "audio/wav",
				},
				body: silentWav(durationSeconds),
			});
			return;
		}
		if (path.includes("/waveform")) {
			await fulfillJson({ building: false, progress: 100 });
			return;
		}
		if (path.includes("/events/")) {
			await fulfillJson([]);
			return;
		}

		await fulfillJson(
			{ detail: `Unhandled mock route: ${request.method()} ${path}` },
			404,
		);
	});
}

test("an invalid ?t= deep link does not break the player", async ({ page }) => {
	const pageErrors: string[] = [];
	page.on("pageerror", (error) => pageErrors.push(error.message));
	await mockRecordingApi(page);

	await page.goto(
		`/dashboard/${GUILD_ID}/audio/${CHANNEL_ID}/2026/8/${FILE_NAME}?t=abc`,
	);

	// canplay must run to completion: assigning NaN to currentTime throws and
	// would otherwise leave the view stuck on "Loading Audio" forever.
	await expect(page.getByText("Loading Audio")).toHaveCount(0, {
		timeout: 15_000,
	});
	await expect(page.getByRole("slider").first()).toBeVisible();
	expect(pageErrors).toEqual([]);
});

test("a valid ?t= deep link still seeks", async ({ page }) => {
	await mockRecordingApi(page);
	await page.goto(
		`/dashboard/${GUILD_ID}/audio/${CHANNEL_ID}/2026/8/${FILE_NAME}?t=0.5`,
	);
	const position = page.getByRole("slider").first();
	await expect(position).toBeVisible({ timeout: 15_000 });
	await expect
		.poll(async () => Number(await position.inputValue()))
		.toBeGreaterThan(0);
});

test("recording controls issue each media command once under StrictMode", async ({
	page,
}) => {
	const errors: string[] = [];
	page.on("pageerror", (error) => errors.push(error.message));
	page.on("console", (message) => {
		if (
			message.type() === "error" &&
			message.text().includes("Cannot update")
		) {
			errors.push(message.text());
		}
	});
	await page.addInitScript((fileName) => {
		const commands: string[] = [];
		Object.defineProperty(window, "recordingMediaCommands", {
			value: commands,
		});
		const prototype = HTMLMediaElement.prototype;
		const originalPlay = prototype.play;
		const originalPause = prototype.pause;
		const currentTime = Object.getOwnPropertyDescriptor(
			prototype,
			"currentTime",
		);
		prototype.play = function () {
			if (this.src.endsWith(`${fileName}.ogg`)) commands.push("play");
			return originalPlay.call(this);
		};
		prototype.pause = function () {
			if (this.src.endsWith(`${fileName}.ogg`)) commands.push("pause");
			return originalPause.call(this);
		};
		Object.defineProperty(prototype, "currentTime", {
			...currentTime,
			set(this: HTMLMediaElement, value: number) {
				if (this.src.endsWith(`${fileName}.ogg`)) commands.push("seek");
				currentTime?.set?.call(this, value);
			},
		});
	}, FILE_NAME);
	await mockRecordingApi(page, 30);
	await page.goto(
		`/dashboard/${GUILD_ID}/audio/${CHANNEL_ID}/2026/8/${FILE_NAME}`,
	);
	await expect(
		page.getByRole("button", { name: "Play", exact: true }),
	).toBeVisible();
	await page.evaluate(() => {
		Reflect.get(window, "recordingMediaCommands").length = 0;
	});
	const commands = () =>
		page.evaluate(
			() => Reflect.get(window, "recordingMediaCommands") as string[],
		);

	await page.getByRole("button", { name: "Play", exact: true }).click();
	await expect(
		page.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();
	await expect.poll(commands).toEqual(["play"]);
	await page.getByRole("button", { name: "Pause", exact: true }).click();
	await expect.poll(commands).toEqual(["play", "pause"]);

	await page.evaluate(() => (document.activeElement as HTMLElement)?.blur());
	await page.keyboard.press("ArrowRight");
	await expect.poll(commands).toEqual(["play", "pause", "seek"]);
	await page.keyboard.press("ArrowLeft");
	await expect.poll(commands).toEqual(["play", "pause", "seek", "seek"]);
	await page.keyboard.press("Space");
	await expect(
		page.getByRole("button", { name: "Pause", exact: true }),
	).toBeVisible();
	await page.keyboard.press("Space");
	await expect(
		page.getByRole("button", { name: "Play", exact: true }),
	).toBeVisible();
	await expect
		.poll(commands)
		.toEqual(["play", "pause", "seek", "seek", "play", "pause"]);
	expect(errors).toEqual([]);
});
