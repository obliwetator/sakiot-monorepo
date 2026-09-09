import AxeBuilder from "@axe-core/playwright";
import { expect, type Page, test } from "@playwright/test";

const API_ORIGIN = "http://127.0.0.1:4174";
const API_PREFIX = "/api";
const GUILD_ID = "guild-123";
// 2^53 + 1: exceeds Number.MAX_SAFE_INTEGER, so it must stay a string.
const SNOWFLAKE_USER_ID = "9007199254740993";
const updateDepthErrors = new WeakMap<Page, string[]>();

test.beforeEach(({ page }) => {
	const errors: string[] = [];
	updateDepthErrors.set(page, errors);
	page.on("console", (message) => {
		if (
			message.type() === "error" &&
			message.text().includes("Maximum update depth exceeded")
		) {
			errors.push(message.text());
		}
	});
	page.on("pageerror", (error) => {
		if (error.message.includes("Maximum update depth exceeded")) {
			errors.push(error.message);
		}
	});
});

test.afterEach(({ page }) => {
	expect(updateDepthErrors.get(page) ?? []).toEqual([]);
});

interface Override {
	user_id: string;
	cooldown_seconds: number;
	updated_at: string;
}

interface MockOptions {
	overrides?: Override[];
	responseDelayMs?: number;
	mutationDelayMs?: number;
}

interface MockState {
	guildCooldown: number;
	overrides: Override[];
	failNextGuildSave: boolean;
	failNextOverrideSave: boolean;
	failNextDelete: boolean;
}

const populatedOverrides: Override[] = [
	{
		user_id: SNOWFLAKE_USER_ID,
		cooldown_seconds: 30,
		updated_at: "2026-08-11T12:30:00.000Z",
	},
	{
		user_id: "10002",
		cooldown_seconds: 0,
		updated_at: "2026-08-10T09:15:00.000Z",
	},
];

const corsHeaders = {
	"Access-Control-Allow-Credentials": "true",
	"Access-Control-Allow-Headers": "Content-Type, X-CSRF-Token",
	"Access-Control-Allow-Methods": "GET, PUT, DELETE, OPTIONS",
	"Access-Control-Allow-Origin": "http://127.0.0.1:4173",
};

async function mockApi(
	page: Page,
	options: MockOptions = {},
): Promise<MockState> {
	const state: MockState = {
		guildCooldown: 10,
		overrides: structuredClone(options.overrides ?? populatedOverrides),
		failNextGuildSave: false,
		failNextOverrideSave: false,
		failNextDelete: false,
	};

	await page.route(`${API_ORIGIN}/**`, async (route) => {
		const request = route.request();
		const url = new URL(request.url());
		const path = url.pathname.replace(API_PREFIX, "");
		const method = request.method();

		if (method === "OPTIONS") {
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

		if (
			options.responseDelayMs &&
			method === "GET" &&
			path.includes("/cooldown")
		) {
			await new Promise((resolve) =>
				setTimeout(resolve, options.responseDelayMs),
			);
		}

		const cooldownPath = `/admin/guilds/${GUILD_ID}/cooldown`;
		const overridesPath = `${cooldownPath}/overrides`;
		if (path === cooldownPath && method === "GET") {
			await fulfillJson({ cooldown_seconds: state.guildCooldown });
			return;
		}
		if (path === overridesPath && method === "GET") {
			await fulfillJson(state.overrides);
			return;
		}
		if (
			path === `/admin/guilds/${GUILD_ID}/voice-settings` &&
			method === "GET"
		) {
			await fulfillJson({ pending_cap_seconds: 21_600, is_default: true });
			return;
		}

		if (options.mutationDelayMs && method !== "GET") {
			await new Promise((resolve) =>
				setTimeout(resolve, options.mutationDelayMs),
			);
		}

		if (path === cooldownPath && method === "PUT") {
			if (state.failNextGuildSave) {
				state.failNextGuildSave = false;
				await fulfillJson({ detail: "save failed" }, 500);
				return;
			}
			const body = request.postDataJSON() as { cooldown_seconds: number };
			state.guildCooldown = body.cooldown_seconds;
			await fulfillJson({});
			return;
		}

		const overrideMatch = path.match(new RegExp(`^${overridesPath}/([^/]+)$`));
		if (overrideMatch && method === "PUT") {
			if (state.failNextOverrideSave) {
				state.failNextOverrideSave = false;
				await fulfillJson({ detail: "save failed" }, 500);
				return;
			}
			const userId = overrideMatch[1];
			const body = request.postDataJSON() as { cooldown_seconds: number };
			const existing = state.overrides.find(
				(override) => override.user_id === userId,
			);
			if (existing) {
				existing.cooldown_seconds = body.cooldown_seconds;
			} else {
				state.overrides.push({
					user_id: userId,
					cooldown_seconds: body.cooldown_seconds,
					updated_at: "2026-08-11T18:00:00.000Z",
				});
			}
			await fulfillJson({});
			return;
		}
		if (overrideMatch && method === "DELETE") {
			if (state.failNextDelete) {
				state.failNextDelete = false;
				await fulfillJson({ detail: "delete failed" }, 500);
				return;
			}
			const userId = overrideMatch[1];
			state.overrides = state.overrides.filter(
				(override) => override.user_id !== userId,
			);
			await fulfillJson({});
			return;
		}

		await fulfillJson(
			{ detail: `Unhandled mock route: ${method} ${path}` },
			404,
		);
	});

	return state;
}

async function openCooldowns(page: Page) {
	await page.goto(`/dashboard/${GUILD_ID}/admin/cooldowns`);
	await expect(
		page.getByRole("heading", { level: 1, name: "Jam cooldowns" }),
	).toBeVisible();
	// The heading renders before the fetched default is initialized. Wait for
	// that value so initialization cannot race Chromium's select-and-fill step.
	await expect(page.getByLabel("Cooldown (seconds)").first()).toHaveValue("10");
}

test("account menu and server picker support keyboard dismissal and restore focus", async ({
	page,
	isMobile,
}, testInfo) => {
	await mockApi(page);
	await openCooldowns(page);
	const trigger = page.getByRole("button", { name: "Open settings" });
	await trigger.focus();
	await trigger.press("Enter");
	await expect(page.getByRole("menu", { name: "Open settings" })).toBeVisible();
	await expect(page.getByRole("menuitem").first()).toBeFocused();
	await page.keyboard.press("Escape");
	await expect(page.getByRole("menu")).toHaveCount(0);
	await expect(trigger).toBeFocused();
	if (isMobile)
		await page.getByRole("button", { name: "open navigation" }).click();
	const picker = page.getByRole("button", { name: /Test Guild Server/ });
	await expect(picker).toHaveCSS("height", "56px");
	const field = await picker.boundingBox();
	const label = await picker
		.locator("..")
		.locator('[data-slot="select-label"]')
		.boundingBox();
	expect(field).not.toBeNull();
	expect(label).not.toBeNull();
	if (!field || !label) throw new Error("Server picker geometry is missing");
	expect(Math.abs(label.y + label.height / 2 - field.y)).toBeLessThanOrEqual(1);
	if (!isMobile) {
		const audio = await page
			.getByRole("button", { name: "Audio", exact: true })
			.boundingBox();
		expect(audio).not.toBeNull();
		if (!audio) throw new Error("Audio navigation is missing");
		expect(
			Math.abs(field.y + field.height / 2 - audio.y - audio.height / 2),
		).toBeLessThanOrEqual(1);
	}
	await picker
		.locator("../..")
		.screenshot({ path: testInfo.outputPath("server-picker.png") });

	await picker.focus();
	await picker.press("ArrowDown");
	await expect(page.getByRole("listbox")).toBeVisible();
	await page.keyboard.press("Escape");
	await expect(page.getByRole("listbox")).toHaveCount(0);
	await expect(picker).toBeFocused();
});

test("shows loading and populated states with accessible keyboard behavior", async ({
	page,
}) => {
	await mockApi(page, { responseDelayMs: 1_500 });
	await page.goto(`/dashboard/${GUILD_ID}/admin/cooldowns`);

	await expect(page.getByText("Loading guild cooldown…")).toBeVisible();
	await expect(page.getByText("Loading admin cooldowns…")).toBeVisible();
	await expect(
		page.getByRole("cell", { name: SNOWFLAKE_USER_ID, exact: true }),
	).toBeVisible();
	await expect(
		page.getByRole("button", {
			name: `Delete override for user ${SNOWFLAKE_USER_ID}`,
		}),
	).toBeVisible();

	const guildInput = page.getByLabel("Cooldown (seconds)").first();
	await guildInput.focus();
	await page.keyboard.press("Tab");
	const saveButton = page.getByRole("button", { name: "Save default" });
	await expect(saveButton).toBeFocused();
	await expect(saveButton).toHaveAttribute("data-focus-visible", "true");
	const outlineStyle = await saveButton.evaluate(
		(element) => getComputedStyle(element).outlineStyle,
	);
	expect(outlineStyle).not.toBe("none");

	await page.keyboard.press("Tab");
	await expect(page.getByLabel("User ID")).toBeFocused();

	const results = await new AxeBuilder({ page }).include("main").analyze();
	const highImpactViolations = results.violations.filter(
		(violation) =>
			violation.impact === "serious" || violation.impact === "critical",
	);
	expect(highImpactViolations).toEqual([]);
});

test("validates native forms, submits with Enter, exposes pending state, and resets successful overrides", async ({
	page,
}) => {
	const state = await mockApi(page, { mutationDelayMs: 300 });
	await openCooldowns(page);

	const guildInput = page.getByLabel("Cooldown (seconds)").first();
	await guildInput.fill("-1");
	await expect(guildInput).toHaveValue("-1");
	await guildInput.press("Enter");
	await expect(page.getByRole("alert")).toContainText(
		"Cooldown must be a non-negative integer.",
	);

	await guildInput.fill("12");
	await guildInput.press("Enter");
	const saveButton = page.getByRole("button", { name: "Save default" });
	await expect(saveButton).toHaveAttribute("data-pending", "true");
	await expect(saveButton).toHaveAttribute("aria-disabled", "true");
	await expect(
		page.getByRole("status").filter({ hasText: "Guild default saved." }),
	).toBeVisible();
	expect(state.guildCooldown).toBe(12);

	const userIdInput = page.getByLabel("User ID");
	const userSecondsInput = page.getByLabel("Cooldown (seconds)").nth(1);
	await userSecondsInput.fill("-2");
	await userSecondsInput.press("Enter");
	await expect(page.getByRole("alert")).toContainText(
		"Provide a user id and non-negative integer seconds.",
	);

	await userIdInput.fill("10003");
	await userSecondsInput.fill("45");
	await userSecondsInput.press("Enter");
	await expect(
		page.getByRole("button", { name: "Add / Update" }),
	).toHaveAttribute("data-pending", "true");
	await expect(
		page.getByRole("status").filter({ hasText: "User override saved." }),
	).toBeVisible();
	await expect(userIdInput).toHaveValue("");
	await expect(userSecondsInput).toHaveValue("0");
	await expect(
		page.getByRole("cell", { name: "10003", exact: true }),
	).toBeVisible();
});

test("announces failed saves and keeps override values available for retry", async ({
	page,
}) => {
	const state = await mockApi(page);
	await openCooldowns(page);

	state.failNextGuildSave = true;
	const guildInput = page.getByLabel("Cooldown (seconds)").first();
	await guildInput.fill("22");
	await guildInput.press("Enter");
	await expect(
		page
			.getByRole("alert")
			.filter({ hasText: "Could not save the guild default." }),
	).toBeVisible();
	state.failNextOverrideSave = true;
	await page.getByLabel("User ID").fill("10004");
	await page.getByLabel("Cooldown (seconds)").nth(1).fill("25");
	await page.getByRole("button", { name: "Add / Update" }).click();
	await expect(
		page
			.getByRole("alert")
			.filter({ hasText: "Could not save the user override." }),
	).toBeVisible();
	await expect(page.getByLabel("User ID")).toHaveValue("10004");
	await expect(page.getByLabel("Cooldown (seconds)").nth(1)).toHaveValue("25");
});

test("announces failed deletion and removes an override after success", async ({
	page,
}) => {
	const state = await mockApi(page);
	await openCooldowns(page);

	state.failNextDelete = true;
	const deleteButton = page.getByRole("button", {
		name: `Delete override for user ${SNOWFLAKE_USER_ID}`,
	});
	await deleteButton.click();
	await expect(page.getByRole("alert")).toContainText(
		`Could not delete the override for user ${SNOWFLAKE_USER_ID}.`,
	);
	await expect(
		page.getByRole("cell", { name: SNOWFLAKE_USER_ID, exact: true }),
	).toBeVisible();

	await deleteButton.click();
	await expect(
		page
			.getByRole("status")
			.filter({ hasText: `Override for user ${SNOWFLAKE_USER_ID} deleted.` }),
	).toBeVisible();
	await expect(
		page.getByRole("cell", { name: SNOWFLAKE_USER_ID, exact: true }),
	).toHaveCount(0);
});

test("renders the empty table state", async ({ page }) => {
	await mockApi(page, { overrides: [] });
	await openCooldowns(page);
	await expect(page.getByText("No per-user overrides.")).toBeVisible();
});

test("shows a snowflake user id in full and scrolls the table instead of clipping it", async ({
	page,
}) => {
	await mockApi(page);
	await openCooldowns(page);

	// The 16-digit id exceeds 2^53 and must render verbatim, not rounded.
	const cell = page.getByRole("cell", {
		name: SNOWFLAKE_USER_ID,
		exact: true,
	});
	await expect(cell).toBeVisible();
	await expect(cell).toHaveText(SNOWFLAKE_USER_ID);

	// The id widens the first column; the table is a horizontal scroll region,
	// so the trailing columns stay reachable rather than being cut off.
	const container = page
		.getByRole("table", { name: "Per-user cooldown overrides" })
		.locator("..");
	// A programmatic scrollLeft only proves the content can move; the styling
	// assertion proves the region is scrollable for a user at all.
	await expect(container).toHaveCSS("overflow-x", "auto");
	const metrics = await container.evaluate((element) => ({
		clientWidth: element.clientWidth,
		scrollWidth: element.scrollWidth,
	}));
	if (metrics.scrollWidth > metrics.clientWidth) {
		await container.evaluate((element) => {
			element.scrollLeft = element.scrollWidth;
		});
		await expect(
			page.getByRole("columnheader", { name: "Updated" }),
		).toBeInViewport();
	}
});

test("navigates between admin screens without an update loop", async ({
	page,
}, testInfo) => {
	const navigateFromShell = async (name: "Admin" | "Voice Settings") => {
		if (testInfo.project.name === "mobile-chromium") {
			await page.getByRole("button", { name: "open navigation" }).click();
		}
		await page.getByRole("button", { name, exact: true }).click();
	};

	await mockApi(page);
	await openCooldowns(page);
	await navigateFromShell("Voice Settings");
	await expect(page).toHaveURL(`/dashboard/${GUILD_ID}/admin/voice-settings`);
	await expect(
		page.getByRole("heading", { name: "Voice Settings" }),
	).toBeVisible();

	await navigateFromShell("Admin");
	await expect(page).toHaveURL(`/dashboard/${GUILD_ID}/admin/cooldowns`);
	await expect(
		page.getByRole("heading", { level: 1, name: "Jam cooldowns" }),
	).toBeVisible();
});

test("@visual matches the cooldowns visual baseline", async ({
	page,
}, testInfo) => {
	await mockApi(page);
	await openCooldowns(page);

	const appBar = page.locator("header").first();
	const toolbar = appBar.locator("div").first();
	await expect(appBar).toBeVisible();
	await expect(toolbar).toBeVisible();
	const toolbarHeight = await toolbar.evaluate(
		(element) => element.getBoundingClientRect().height,
	);
	expect(toolbarHeight).toBeGreaterThanOrEqual(56);

	const shellControl =
		testInfo.project.name === "desktop-chromium"
			? page.getByRole("button", { name: "Audio" })
			: page.getByRole("button", { name: "open navigation" });
	const horizontalPadding = await shellControl.evaluate((element) => {
		const style = getComputedStyle(element);
		return (
			Number.parseFloat(style.paddingLeft) +
			Number.parseFloat(style.paddingRight)
		);
	});
	expect(horizontalPadding).toBeGreaterThan(0);

	await expect(page).toHaveScreenshot("cooldowns-shell.png", {
		animations: "disabled",
	});
	await expect(page.getByRole("main")).toHaveScreenshot("cooldowns-page.png", {
		animations: "disabled",
	});
});
