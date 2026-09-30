import {
	draftRecord,
	GENERIC_DRAFT_KEY,
	GUILD_ID,
	mockClipEditorApi,
	singleSegmentComposition,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

test("oversized browser preview is explained while export stays available", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.addInitScript(
		({ key, record }) => localStorage.setItem(key, record),
		{
			key: GENERIC_DRAFT_KEY,
			record: draftRecord(singleSegmentComposition(180)),
		},
	);
	await page.goto(`/dashboard/${GUILD_ID}/clips/editor`);
	await expect(
		page.getByText("This edit is too large for browser preview.", {
			exact: false,
		}),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Play", exact: true }),
	).toBeDisabled();
	await page.keyboard.press("Space");
	await expect(
		page.getByRole("button", { name: "Pause", exact: true }),
	).toHaveCount(0);
	await expect(
		page.getByRole("button", { name: "Export", exact: true }),
	).toBeEnabled();
});
