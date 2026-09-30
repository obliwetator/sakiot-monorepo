import {
	corsHeaders,
	draftRecord,
	GENERIC_DRAFT_KEY,
	GUILD_ID,
	mockClipEditorApi,
	singleSegmentComposition,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const SOURCE_SECONDS = 2.5;

/** A mono 16-bit PCM WAV of silence, long enough to pause part-way. */
function silence(seconds: number): Buffer {
	const sampleRate = 8_000;
	const dataBytes = Math.round(seconds * sampleRate) * 2;
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

test("pausing keeps the playhead, and playing to the end rewinds it", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.route(
		`**/api/audio/clips/${GUILD_ID}/working-source`,
		async (route) => {
			await route.fulfill({
				status: 200,
				headers: { ...corsHeaders, "Content-Type": "audio/wav" },
				body: silence(SOURCE_SECONDS),
			});
		},
	);
	await page.addInitScript(
		({ key, record }) => localStorage.setItem(key, record),
		{
			key: GENERIC_DRAFT_KEY,
			record: draftRecord(singleSegmentComposition(SOURCE_SECONDS)),
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);

	const play = page.getByRole("button", { name: "Play", exact: true });
	const pause = page.getByRole("button", { name: "Pause", exact: true });
	const playhead = page.getByText(/^\d\d:\d\d:\d\d \/ 00:00:02$/);

	await play.click();
	await expect(pause).toBeVisible();
	await expect(playhead).toHaveText(/^00:00:01 /, { timeout: 5_000 });

	// A frame already queued when pausing must not rewind the playhead.
	await pause.click();
	await expect(play).toBeVisible();
	await page.waitForTimeout(300);
	await expect(playhead).toHaveText(/^00:00:01 /);

	// Resuming continues from there and reaches the end, which rewinds.
	await play.click();
	await expect(pause).toBeVisible();
	await expect(play).toBeVisible({ timeout: 5_000 });
	await expect(playhead).toHaveText(/^00:00:00 /);
});
