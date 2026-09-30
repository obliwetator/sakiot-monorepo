import { readFile } from "node:fs/promises";
import type { Page } from "@playwright/test";
import { expectAccessibleMediaRoute } from "./axe";
import {
	draftKey,
	draftRecord,
	GENERIC_DRAFT_KEY,
	GUILD_ID,
	mockClipEditorApi,
	singleSegmentComposition,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const EDITOR = `/dashboard/${GUILD_ID}/clips/editor`;
const SOURCE_EDITOR = `${EDITOR}?source=working-source`;

test.beforeEach(async ({ page }) => {
	await mockClipEditorApi(page);
});

function segmentCount(page: Page, count: number) {
	return page.getByText(`${count} segment${count === 1 ? "" : "s"}`, {
		exact: true,
	});
}

function saveStatus(page: Page, text: string | RegExp) {
	return page.getByRole("status").filter({ hasText: text });
}

/** Seeds the generic draft once, so reloads see what the editor saved. */
async function seedGenericDraft(page: Page, record: string) {
	await page.addInitScript(
		({ key, value }) => {
			if (sessionStorage.getItem("test:draft-seeded")) return;
			localStorage.setItem(key, value);
			sessionStorage.setItem("test:draft-seeded", "yes");
		},
		{ key: GENERIC_DRAFT_KEY, value: record },
	);
}

/** Makes every draft write throw a quota error until `allowDraftWrites`. */
async function failDraftWrites(page: Page) {
	await page.addInitScript(() => {
		const setItem = Storage.prototype.setItem;
		Reflect.set(window, "__failDraftWrites", true);
		Storage.prototype.setItem = function (key: string, value: string) {
			if (
				key.startsWith("sakiot:clip-editor:drafts:") &&
				Reflect.get(window, "__failDraftWrites") === true
			) {
				throw new DOMException(
					"The quota has been exceeded.",
					"QuotaExceededError",
				);
			}
			setItem.call(this, key, value);
		};
	});
}

async function allowDraftWrites(page: Page) {
	await page.evaluate(() => Reflect.set(window, "__failDraftWrites", false));
}

async function storedSegments(page: Page, key: string) {
	return page.evaluate((storageKey) => {
		const raw = localStorage.getItem(storageKey);
		if (raw === null) return null;
		try {
			return JSON.parse(raw).composition.segments.length as number;
		} catch {
			return raw;
		}
	}, key);
}

/** Opens a page from the navbar, through the mobile drawer when collapsed. */
async function navigateTo(page: Page, name: string) {
	const inline = page.getByRole("button", { name, exact: true }).first();
	if (!(await inline.isVisible())) {
		await page.getByRole("button", { name: "open navigation" }).click();
	}
	await page.getByRole("button", { name, exact: true }).first().click();
}

async function deleteSelectedSegment(page: Page) {
	await expect(page.getByTestId("clip-inspector")).not.toContainText(
		"No segment selected.",
	);
	await page.keyboard.press("Delete");
}

test("deleting the final segment stays deleted after a reload", async ({
	page,
}) => {
	await page.goto(SOURCE_EDITOR);
	await expect(segmentCount(page, 1)).toBeVisible();
	await deleteSelectedSegment(page);
	await expect(segmentCount(page, 0)).toBeVisible();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();

	const clipsLoaded = page.waitForResponse(
		(response) =>
			new URL(response.url()).pathname === `/api/audio/clips/${GUILD_ID}`,
	);
	await page.reload();
	await clipsLoaded;
	await expect(segmentCount(page, 0)).toBeVisible();
	// The clip list has arrived, so a wrong reseed would have happened by now.
	await page.waitForTimeout(300);
	await expect(segmentCount(page, 0)).toBeVisible();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
});

test("each source clip and the generic editor restore their own draft", async ({
	page,
}) => {
	await seedGenericDraft(page, draftRecord(singleSegmentComposition(1)));
	await page.goto(SOURCE_EDITOR);
	await expect(segmentCount(page, 1)).toBeVisible();
	await deleteSelectedSegment(page);
	await expect(segmentCount(page, 0)).toBeVisible();

	// Saved work leaves without a dialog, and the generic draft is untouched.
	await navigateTo(page, "Clip Editor");
	await expect(page).toHaveURL(new RegExp(`${EDITOR}$`));
	await expect(segmentCount(page, 1)).toBeVisible();

	// Rapid history navigation lands on each context's own draft.
	await page.goBack();
	await page.goForward();
	await page.goBack();
	await expect(page).toHaveURL(/source=working-source/);
	await expect(segmentCount(page, 0)).toBeVisible();
	await page.goForward();
	await expect(segmentCount(page, 1)).toBeVisible();

	expect(await storedSegments(page, GENERIC_DRAFT_KEY)).toBe(1);
	expect(await storedSegments(page, draftKey("working-source"))).toBe(0);
});

test("undo, redo and restore-original are each saved", async ({ page }) => {
	await page.goto(SOURCE_EDITOR);
	await expect(segmentCount(page, 1)).toBeVisible();
	const key = draftKey("working-source");
	await deleteSelectedSegment(page);
	await expect.poll(() => storedSegments(page, key)).toBe(0);
	await page.getByRole("button", { name: "Undo (Ctrl+Z)" }).click();
	await expect(segmentCount(page, 1)).toBeVisible();
	await expect.poll(() => storedSegments(page, key)).toBe(1);
	await page.getByRole("button", { name: "Redo (Ctrl+Shift+Z)" }).click();
	await expect.poll(() => storedSegments(page, key)).toBe(0);
	await page.getByRole("button", { name: "Restore original clip" }).click();
	await expect(segmentCount(page, 1)).toBeVisible();
	await expect.poll(() => storedSegments(page, key)).toBe(1);
	await page.reload();
	await expect(segmentCount(page, 1)).toBeVisible();
});

test("a failed save is never shown as saved, and retry saves the latest edit", async ({
	page,
}) => {
	await failDraftWrites(page);
	await page.goto(SOURCE_EDITOR);
	await expect(segmentCount(page, 1)).toBeVisible();
	await deleteSelectedSegment(page);

	await expect(saveStatus(page, "Couldn't save this draft")).toBeVisible();
	await expect(saveStatus(page, "Saved")).toHaveCount(0);
	const failure = page.getByRole("alert").filter({
		hasText: "This browser's storage for the site is full.",
	});
	await expect(failure).toBeVisible();
	await expectAccessibleMediaRoute(page);

	// Leaving asks first, and staying keeps the page.
	await navigateTo(page, "Clip Editor");
	const dialog = page.getByRole("dialog", {
		name: "Leave without saving this draft?",
	});
	await expect(dialog).toBeVisible();
	await dialog.getByRole("button", { name: "Stay" }).click();
	await expect(dialog).toBeHidden();

	await allowDraftWrites(page);
	await failure.getByRole("button", { name: "Retry" }).click();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
	await expect(failure).toBeHidden();
	await page.reload();
	await expect(segmentCount(page, 0)).toBeVisible();
});

test("a downloaded draft opens again from the editor options", async ({
	page,
}) => {
	await seedGenericDraft(page, draftRecord(singleSegmentComposition(1)));
	await failDraftWrites(page);
	await page.goto(EDITOR);
	await deleteSelectedSegment(page);
	const failure = page.getByRole("alert").filter({ hasText: "Couldn't save" });
	const downloading = page.waitForEvent("download");
	await failure.getByRole("button", { name: "Download draft" }).click();
	const download = await downloading;
	expect(download.suggestedFilename()).toMatch(
		/^sakiot-clip-draft-editor-.+\.json$/,
	);
	const file = await download.path();
	const saved = JSON.parse(await readFile(file, "utf8"));
	expect(saved).toMatchObject({
		kind: "sakiot-clip-editor-draft",
		guild_id: GUILD_ID,
		composition: { segments: [] },
	});

	// The failed delete never reached storage, so a reload shows the segment.
	await page.reload();
	await allowDraftWrites(page);
	await expect(segmentCount(page, 1)).toBeVisible();

	await page.getByRole("button", { name: "Editor options (Ctrl+,)" }).click();
	const choosing = page.waitForEvent("filechooser");
	await page.getByRole("button", { name: "Open draft file…" }).click();
	await (await choosing).setFiles(file);
	await expect(
		page.getByRole("dialog", { name: "Editor options" }),
	).toBeHidden();
	await expect(segmentCount(page, 0)).toBeVisible();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
});

test("two tabs cannot silently overwrite each other's draft", async ({
	page,
}) => {
	await seedGenericDraft(page, draftRecord(singleSegmentComposition(1)));
	await page.goto(EDITOR);
	await expect(segmentCount(page, 1)).toBeVisible();

	const other = await page.context().newPage();
	await mockClipEditorApi(other);
	await other.goto(EDITOR);
	await expect(segmentCount(other, 1)).toBeVisible();

	await deleteSelectedSegment(page);
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();

	const conflict = other.getByRole("alert").filter({
		hasText: "This draft was changed in another tab.",
	});
	await expect(conflict).toBeVisible();
	await expect(saveStatus(other, "Saving paused")).toBeVisible();
	await other.getByRole("button", { name: "+ Track" }).click();
	expect(await storedSegments(other, GENERIC_DRAFT_KEY)).toBe(0);

	await conflict
		.getByRole("button", { name: "Load other tab's version" })
		.click();
	await expect(segmentCount(other, 0)).toBeVisible();
	await expect(saveStatus(other, "Saved on this device")).toBeVisible();
	await other.close();
});

test("an unreadable stored draft is reported and kept until replaced", async ({
	page,
}) => {
	await seedGenericDraft(page, "{not json");
	await page.goto(EDITOR);
	const damaged = page.getByRole("alert").filter({
		hasText: "The draft saved on this device can't be read.",
	});
	await expect(damaged).toBeVisible();
	await page.getByRole("button", { name: "+ Track" }).click();
	await expect(saveStatus(page, "Saving paused")).toBeVisible();
	expect(await storedSegments(page, GENERIC_DRAFT_KEY)).toBe("{not json");

	const downloading = page.waitForEvent("download");
	await damaged.getByRole("button", { name: "Download saved data" }).click();
	expect(await readFile(await (await downloading).path(), "utf8")).toBe(
		"{not json",
	);

	await damaged
		.getByRole("button", { name: "Replace it with this edit" })
		.click();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
	expect(await storedSegments(page, GENERIC_DRAFT_KEY)).toBe(0);
});

test("a draft from the earlier editor is offered, not applied", async ({
	page,
}) => {
	await page.addInitScript(
		({ key, value }) => localStorage.setItem(key, value),
		{
			key: `sakiot:clip-editor:${GUILD_ID}:draft`,
			value: JSON.stringify(singleSegmentComposition(1)),
		},
	);
	await page.goto(EDITOR);
	const offer = page.getByRole("status").filter({
		hasText: "This device has a draft from the earlier editor.",
	});
	await expect(offer).toBeVisible();
	await expect(segmentCount(page, 0)).toBeVisible();
	await offer.getByRole("button", { name: "Load earlier draft" }).click();
	await expect(segmentCount(page, 1)).toBeVisible();
	await expect(offer).toBeHidden();
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
});

test("closing the tab asks only while the draft is unsaved", async ({
	page,
}) => {
	await seedGenericDraft(page, draftRecord(singleSegmentComposition(1)));
	await failDraftWrites(page);
	await page.goto(EDITOR);
	await deleteSelectedSegment(page);
	await expect(saveStatus(page, "Couldn't save this draft")).toBeVisible();
	const dialogs: string[] = [];
	page.on("dialog", (dialog) => {
		dialogs.push(dialog.type());
		void dialog.dismiss();
	});
	await page.close({ runBeforeUnload: true });
	await expect.poll(() => dialogs).toEqual(["beforeunload"]);
});

test("closing a saved draft's tab does not ask", async ({ page }) => {
	await page.goto(SOURCE_EDITOR);
	await deleteSelectedSegment(page);
	await expect(saveStatus(page, "Saved on this device")).toBeVisible();
	const dialogs: string[] = [];
	page.on("dialog", (dialog) => {
		dialogs.push(dialog.type());
		void dialog.accept();
	});
	await page.close({ runBeforeUnload: true });
	await expect.poll(() => page.isClosed()).toBe(true);
	expect(dialogs).toEqual([]);
});
