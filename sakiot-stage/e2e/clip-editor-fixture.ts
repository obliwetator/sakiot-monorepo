import type { Page } from "@playwright/test";

export const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
export const GUILD_ID = "guild-123";

export const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token, Idempotency-Key",
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

export async function mockClipEditorApi(page: Page) {
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
