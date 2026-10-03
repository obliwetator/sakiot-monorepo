import type { Page } from "@playwright/test";
import {
	API_ORIGIN,
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const audioPath = `/dashboard/${GUILD_ID}/audio`;
const treePath = `/api/current/${GUILD_ID}`;
const modulePattern =
	/\/(?:src\/features\/audio-dashboard\/YearSelection\.tsx|assets\/YearSelection-[^/]+\.js)(?:\?|$)/;

/** The recording tree and its live stems (not the voice presence panel). */
function isTreeRequest(pathname: string): boolean {
	return pathname === treePath || pathname === `${treePath}/live-stems`;
}

function gate() {
	let release!: () => void;
	const promise = new Promise<void>((resolve) => {
		release = resolve;
	});
	return { promise, release };
}

async function showTree(page: Page) {
	const browse = page.getByRole("button", { name: "Browse files" });
	await expect(
		browse.or(page.getByRole("textbox", { name: "Search recordings" })),
	).toBeVisible();
	if (await browse.isVisible()) await browse.click();
	await expect(
		page.getByRole("textbox", { name: "Search recordings" }),
	).toBeVisible();
}

test("authorized data loads while the audio module is held, without a mount refetch", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	const moduleGate = gate();
	const authGate = gate();
	const moduleStarted = gate();
	const authStarted = gate();
	const requests: string[] = [];
	const authRequests: string[] = [];
	page.on("request", (request) => {
		const url = new URL(request.url());
		if (isTreeRequest(url.pathname)) requests.push(url.pathname);
		if (
			["/api/users/current", "/api/users/current/guilds"].includes(url.pathname)
		)
			authRequests.push(url.pathname);
	});
	await page.route(modulePattern, async (route) => {
		moduleStarted.release();
		await moduleGate.promise;
		await route.continue();
	});
	await page.route(`${API_ORIGIN}/api/users/current/guilds`, async (route) => {
		authStarted.release();
		await authGate.promise;
		await route.fallback();
	});
	try {
		await page.goto(audioPath, { waitUntil: "commit" });
		await Promise.all([moduleStarted.promise, authStarted.promise]);
		expect(requests).toEqual([]);
		const finishedTree = page.waitForResponse(
			(response) => new URL(response.url()).pathname === treePath,
		);
		authGate.release();
		await (await finishedTree).finished();
		await expect
			.poll(() => requests)
			.toEqual([treePath, `${treePath}/live-stems`]);
		await expect(page.getByRole("status")).toHaveText("Loading Route");
		moduleGate.release();
		await showTree(page);
		expect(requests).toEqual([treePath, `${treePath}/live-stems`]);
		expect(authRequests.sort()).toEqual([
			"/api/users/current",
			"/api/users/current/guilds",
		]);
	} finally {
		authGate.release();
		moduleGate.release();
	}
});

test("a denied guild does not start recording requests", async ({ page }) => {
	await mockClipEditorApi(page);
	const requests: string[] = [];
	page.on("request", (request) => {
		if (new URL(request.url()).pathname.startsWith("/api/current/"))
			requests.push(request.url());
	});
	await page.goto("/dashboard/unknown-guild/audio");
	await expect(
		page.getByRole("heading", { name: "Server access unavailable" }),
	).toBeVisible();
	expect(requests).toEqual([]);
});

test("role changes and return visits refresh each query once", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	await page.route(
		`${API_ORIGIN}/api/admin/guilds/${GUILD_ID}/roles`,
		(route) => route.fulfill({ headers: corsHeaders, json: [] }),
	);
	const requests: string[] = [];
	page.on("request", (request) => {
		const url = new URL(request.url());
		if (isTreeRequest(url.pathname)) requests.push(url.pathname + url.search);
	});
	await page.goto(`${audioPath}?as_role=role-123`);
	await expect(
		page.getByRole("button", { name: "Exit preview" }),
	).toBeVisible();
	await expect.poll(() => requests.length).toBe(2);
	expect(requests.every((url) => url.endsWith("?as_role=role-123"))).toBe(true);
	await page.getByRole("button", { name: "Exit preview" }).click();
	await expect(page).toHaveURL(new RegExp(`${audioPath}$`));
	await showTree(page);
	await expect.poll(() => requests.length).toBe(4);
	expect(requests.slice(2)).toEqual([treePath, `${treePath}/live-stems`]);
	await page.keyboard.press("Escape");
	await page.getByRole("button", { name: "Clips", exact: true }).click();
	await expect(page).toHaveURL(/\/clips$/);
	await page.goBack();
	await showTree(page);
	await expect.poll(() => requests.length).toBe(6);
});

test("login after the initial denied probe reruns the audio loader", async ({
	page,
}) => {
	await mockClipEditorApi(page);
	let loggedIn = false;
	let trees = 0;
	await page.route(`${API_ORIGIN}/api/users/current`, async (route) => {
		if (loggedIn) return route.fallback();
		await route.fulfill({ headers: corsHeaders, json: null });
	});
	page.on("request", (request) => {
		if (new URL(request.url()).pathname === treePath) trees++;
	});
	await page.goto(audioPath);
	await expect(
		page.getByText(
			"You are not logged in or you are not authorized to view this content",
		),
	).toBeVisible();
	expect(trees).toBe(0);
	loggedIn = true;
	await page.evaluate((origin) => {
		window.dispatchEvent(
			new MessageEvent("message", {
				origin,
				data: {
					type: "sakiot-auth",
					success: 1,
					csrf: "test-csrf-token-long-enough",
				},
			}),
		);
	}, API_ORIGIN);
	await showTree(page);
	expect(trees).toBe(1);
});

test("an audio chunk failure has a reload action", async ({
	page,
	consoleAudit,
}) => {
	await mockClipEditorApi(page);
	consoleAudit.allow(
		/net::ERR_FAILED|Failed to fetch dynamically imported module|Importing a module script failed|error occurred in one of your React components/i,
	);
	await page.route(modulePattern, (route) => route.abort("failed"));
	await page.goto(audioPath);
	await expect(
		page.getByRole("heading", { name: "Could not load the audio page" }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Reload", exact: true }),
	).toBeVisible();
});

// Without realtime, an idle tree still checks live stems every 30 s, to
// discover new recordings, but never at the 10 s live rate.
test("an idle recording tree does not poll live stems at the live rate", async ({
	page,
}) => {
	await page.clock.install();
	await mockClipEditorApi(page);
	let liveRequests = 0;
	page.on("request", (request) => {
		if (new URL(request.url()).pathname === `${treePath}/live-stems`)
			liveRequests++;
	});
	await page.goto(audioPath);
	await showTree(page);
	expect(liveRequests).toBe(1);
	// A minute holds six polls at the live rate, two at the idle rate.
	await page.clock.runFor(60_000);
	expect(liveRequests - 1).toBeLessThanOrEqual(2);
});

test("live-stem polling slows down after the last recording ends", async ({
	page,
}) => {
	await page.clock.install();
	await mockClipEditorApi(page);
	let liveRequests = 0;
	await page.route(`${API_ORIGIN}${treePath}/live-stems`, (route) => {
		liveRequests++;
		return route.fulfill({
			headers: corsHeaders,
			json: liveRequests === 1 ? ["active-recording"] : [],
		});
	});
	await page.goto(audioPath);
	await showTree(page);
	expect(liveRequests).toBe(1);
	const finished = page.waitForResponse(
		(response) => new URL(response.url()).pathname === `${treePath}/live-stems`,
	);
	await page.clock.fastForward(10_000);
	await (await finished).finished();
	await expect.poll(() => liveRequests).toBe(2);
	// Let React apply the empty result and remove the subscription's timer.
	await page.clock.runFor(100);
	await page.clock.runFor(60_000);
	expect(liveRequests - 2).toBeLessThanOrEqual(2);
});
