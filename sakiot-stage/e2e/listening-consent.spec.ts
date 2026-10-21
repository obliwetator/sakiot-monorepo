import type { Page } from "@playwright/test";
import {
	API_ORIGIN,
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, fulfillSharedRoutes, test } from "./fixtures";

/**
 * Answers the selected server's listening consent as stored, recording every
 * change; `optOut` answers the opt-out as given, which is refused to former
 * members.
 */
async function mockConsent(
	page: Page,
	optOut: { status: number; body: unknown } = {
		status: 200,
		body: { opted_out: false },
	},
) {
	let keptPrivate = false;
	const saved: unknown[] = [];
	const headers = {
		...corsHeaders,
		"Access-Control-Allow-Methods": "GET, PUT, OPTIONS",
	};
	const json = { ...headers, "Content-Type": "application/json" };
	// Registered after the catch-all mock, so they take precedence.
	await page.route(
		`**/api/users/current/guilds/${GUILD_ID}/recording-privacy`,
		async (route) => {
			const request = route.request();
			if (request.method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers });
				return;
			}
			if (request.method() === "PUT") {
				const body = request.postDataJSON() as { private: boolean };
				saved.push(body);
				keptPrivate = body.private;
			}
			await route.fulfill({
				status: 200,
				headers: json,
				body: JSON.stringify({ private: keptPrivate }),
			});
		},
	);
	await page.route(
		`**/api/users/current/guilds/${GUILD_ID}/recording-opt-out`,
		async (route) => {
			if (route.request().method() === "OPTIONS") {
				await route.fulfill({ status: 204, headers });
				return;
			}
			await route.fulfill({
				status: optOut.status,
				headers: json,
				body: JSON.stringify(optOut.body),
			});
		},
	);
	return saved;
}

async function openUserMenu(page: Page) {
	await page.goto("/dashboard");
	await page.getByRole("button", { name: "Test Guild" }).click();
	await expect(page).toHaveURL(new RegExp(`/dashboard/${GUILD_ID}/audio$`));
	await page.getByRole("button", { name: "Open user menu" }).click();
}

test("members choose whether only they can listen to their recordings", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	const saved = await mockConsent(page);
	await openUserMenu(page);

	const onlyMe = page.getByRole("menuitemcheckbox", {
		name: "Only I can listen to my recordings in Test Guild",
	});
	const recordMe = page.getByRole("menuitemcheckbox", {
		name: "Record my voice in Test Guild",
	});
	// Shared by default, and apart from whether they are recorded.
	await expect(onlyMe).not.toBeChecked();
	await expect(recordMe).toBeChecked();

	await onlyMe.click();
	await expect.poll(() => saved).toEqual([{ private: true }]);
	await expect(onlyMe).toBeChecked();
	await expect(recordMe).toBeChecked();

	await onlyMe.click();
	await expect
		.poll(() => saved)
		.toEqual([{ private: true }, { private: false }]);
	await expect(onlyMe).not.toBeChecked();
});

test("a former member still chooses who listens, though no longer recorded", async ({
	page,
	consoleAudit,
}) => {
	consoleAudit.allow(/status of 403.*\/recording-opt-out/);
	await mockClipEditorApi(page);
	const saved = await mockConsent(page, {
		status: 403,
		body: { code: 403, kind: "forbidden", message: "Forbidden" },
	});
	await openUserMenu(page);

	const onlyMe = page.getByRole("menuitemcheckbox", {
		name: "Only I can listen to my recordings in Test Guild",
	});
	await expect(onlyMe).not.toBeChecked();
	await expect(
		page.getByRole("menuitemcheckbox", {
			name: "Record my voice in Test Guild",
		}),
	).toHaveCount(0);
	await onlyMe.click();
	await expect.poll(() => saved).toEqual([{ private: true }]);
});

const SESSION_ID = "session-123";
const DELETION = {
	attempts: 0,
	id: "deletion-1",
	mode: "soft",
	recording_session_id: SESSION_ID,
	stage: "done",
	state: "soft_deleted",
	status_url: `/api/admin/guilds/${GUILD_ID}/recording-deletions/deletion-1`,
};

test("a manager sees a private recording redacted, only to remove it", async ({
	page,
	consoleAudit,
}) => {
	// Opened directly, the manifest may be asked for before the tree says
	// the session is redacted.
	consoleAudit.allow(/status of 403.*\/manifest/);
	const removed: string[] = [];
	const managerHeaders = {
		...corsHeaders,
		"Access-Control-Allow-Methods": "GET, DELETE, OPTIONS",
	};
	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname.replace("/api", "");
		if (request.method() === "OPTIONS") {
			await route.fulfill({ status: 204, headers: managerHeaders });
			return;
		}
		if (await fulfillSharedRoutes(route, managerHeaders)) return;
		const fulfillJson = async (body: unknown, status = 200) => {
			await route.fulfill({
				status,
				headers: { ...managerHeaders, "Content-Type": "application/json" },
				body: JSON.stringify(body),
			});
		};
		if (path === "/users/current") {
			await fulfillJson({
				avatar: "",
				is_dev: false,
				user_id: "manager",
				username: "Manager",
			});
		} else if (path === "/users/current/guilds") {
			await fulfillJson([
				{
					id: GUILD_ID,
					name: "Test Guild",
					capabilities: ["delete_recordings"],
				},
			]);
		} else if (path === `/current/${GUILD_ID}`) {
			await fulfillJson([
				{
					channel_id: "voice-123",
					dirs: [
						{
							year: 2026,
							months: {
								10: [
									{
										file: "1786460400000-Private.ogg",
										display_name: "Private",
										recording_session_id: SESSION_ID,
										state: "finalized",
										user_id: "private-speaker",
										redacted: true,
									},
								],
							},
						},
					],
				},
			]);
		} else if (path === `/current/${GUILD_ID}/live-stems`) {
			await fulfillJson([]);
		} else if (path === `/audio/sessions/${SESSION_ID}/manifest`) {
			await fulfillJson(
				{ code: 403, kind: "forbidden", message: "Forbidden" },
				403,
			);
		} else if (
			path === `/admin/guilds/${GUILD_ID}/recordings/${SESSION_ID}` &&
			request.method() === "DELETE"
		) {
			removed.push(SESSION_ID);
			await fulfillJson(DELETION, 202);
		} else if (
			path === `/admin/guilds/${GUILD_ID}/recording-deletions/deletion-1`
		) {
			await fulfillJson(DELETION);
		} else {
			await fulfillJson([]);
		}
	});

	await page.goto(`/dashboard/${GUILD_ID}/audio/session/${SESSION_ID}`);
	await expect(
		page.getByText("Its speaker keeps their recordings private"),
	).toBeVisible();
	await expect(page.getByRole("tab", { name: "Normal" })).toHaveCount(0);

	page.once("dialog", (dialog) => void dialog.accept());
	await page
		.getByRole("button", { name: "Remove recording from view" })
		.click();
	await expect(page.getByText("Recording removed from view.")).toBeVisible();
	expect(removed).toEqual([SESSION_ID]);

	// The tree marks it too; narrow screens keep the tree behind a button.
	const browse = page.getByRole("button", {
		name: "Browse files",
		exact: true,
	});
	if (await browse.isVisible()) await browse.click();
	await expect(
		page.getByRole("treegrid", { name: "Recordings" }).getByText("🔒 private"),
	).toBeVisible();
});
