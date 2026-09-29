import type { Locator, Page } from "@playwright/test";
import {
	API_ORIGIN,
	corsHeaders,
	GUILD_ID,
	mockClipEditorApi,
} from "./clip-editor-fixture";
import { expect, test } from "./fixtures";

const guildRoot = `/dashboard/${GUILD_ID}`;
const adminRoot = `/api/admin/guilds/${GUILD_ID}`;
const clipsPath = `/api/audio/clips/${GUILD_ID}`;
const stampsPath = `/api/stamps/${GUILD_ID}`;
const responses: Record<string, unknown> = {
	[stampsPath]: [],
	[`${adminRoot}/cooldown`]: { cooldown_seconds: 10 },
	[`${adminRoot}/cooldown/overrides`]: [],
	[`${adminRoot}/voice-settings`]: {
		pending_cap_seconds: 21600,
		is_default: true,
	},
	[`${adminRoot}/recording-policy`]: {
		retention_days: null,
		excluded_channel_ids: [],
		channels: [],
		is_default: true,
	},
	[`${adminRoot}/roles`]: [],
};

type RouteCase = {
	feature: string;
	path: string;
	queries: string[];
	ready: (page: Page) => Locator;
};
const cases: RouteCase[] = [
	{
		feature: "clips",
		path: `${guildRoot}/clips`,
		queries: [clipsPath],
		ready: (page) =>
			page
				.getByRole("tab", { name: "Clips", exact: true })
				.or(page.getByRole("button", { name: "Browse clips" })),
	},
	{
		feature: "clip-editor",
		path: `${guildRoot}/clips/editor`,
		queries: [clipsPath],
		ready: (page) =>
			page.getByRole("button", {
				name: "Editor options (Ctrl+,)",
				exact: true,
			}),
	},
	{
		feature: "stamps",
		path: `/stamps/${GUILD_ID}`,
		queries: [stampsPath],
		ready: (page) => page.getByRole("heading", { name: /^Stamps/ }),
	},
	{
		feature: "admin-cooldowns",
		path: `${guildRoot}/admin/cooldowns`,
		queries: [`${adminRoot}/cooldown`, `${adminRoot}/cooldown/overrides`],
		ready: (page) => page.getByRole("heading", { name: "Jam cooldowns" }),
	},
	{
		feature: "admin-voice-settings",
		path: `${guildRoot}/admin/voice-settings`,
		queries: [`${adminRoot}/voice-settings`, `${adminRoot}/recording-policy`],
		ready: (page) =>
			page.getByRole("heading", { name: "Voice Settings", exact: true }),
	},
	{
		feature: "members",
		path: `${guildRoot}/members`,
		queries: [`${adminRoot}/roles`],
		ready: (page) => page.getByRole("heading", { name: "Members & roles" }),
	},
];

function gate() {
	let release!: () => void;
	const promise = new Promise<void>((resolve) => {
		release = resolve;
	});
	return { promise, release };
}

function modulePattern(feature: string) {
	return new RegExp(
		`/(?:src/features/${feature}/index\\.tsx|assets/${feature}-[^/]+\\.js)(?:\\?|$)`,
	);
}

async function mockPageApi(page: Page) {
	await mockClipEditorApi(page);
	await page.route(`${API_ORIGIN}/api/**`, async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (route.request().method() !== "GET" || !(path in responses))
			return route.fallback();
		await route.fulfill({ headers: corsHeaders, json: responses[path] });
	});
}

for (const routeCase of cases) {
	test(`${routeCase.feature} fetches authorized data before its module finishes, once`, async ({
		page,
	}) => {
		await mockPageApi(page);
		const moduleGate = gate();
		const authGate = gate();
		const moduleStarted = gate();
		const authStarted = gate();
		const requests: string[] = [];
		const authRequests: string[] = [];
		page.on("request", (request) => {
			const path = new URL(request.url()).pathname;
			if (routeCase.queries.includes(path)) requests.push(path);
			if (["/api/users/current", "/api/users/current/guilds"].includes(path))
				authRequests.push(path);
		});
		await page.route(modulePattern(routeCase.feature), async (route) => {
			moduleStarted.release();
			await moduleGate.promise;
			await route.continue();
		});
		await page.route(
			`${API_ORIGIN}/api/users/current/guilds`,
			async (route) => {
				authStarted.release();
				await authGate.promise;
				await route.fallback();
			},
		);
		try {
			await page.goto(routeCase.path, { waitUntil: "commit" });
			await Promise.all([moduleStarted.promise, authStarted.promise]);
			expect(requests).toEqual([]);
			const complete = Promise.all(
				routeCase.queries.map(async (path) => {
					const response = await page.waitForResponse(
						(response) => new URL(response.url()).pathname === path,
					);
					await response.finished();
				}),
			);
			authGate.release();
			await complete;
			await expect(page.getByRole("status")).toHaveText("Loading Route");
			moduleGate.release();
			await expect(routeCase.ready(page)).toBeVisible();
			expect(requests.sort()).toEqual([...routeCase.queries].sort());
			expect(authRequests.sort()).toEqual([
				"/api/users/current",
				"/api/users/current/guilds",
			]);
		} finally {
			authGate.release();
			moduleGate.release();
		}
	});

	test(`${routeCase.feature} does not preload data for a denied guild`, async ({
		page,
	}) => {
		await mockPageApi(page);
		const requests: string[] = [];
		page.on("request", (request) => {
			const url = new URL(request.url());
			if (url.origin === API_ORIGIN && url.pathname.includes("denied-guild"))
				requests.push(url.pathname);
		});
		await page.goto(routeCase.path.replace(GUILD_ID, "denied-guild"));
		await expect(
			page.getByRole("heading", { name: "Server access unavailable" }),
		).toBeVisible();
		expect(requests).toEqual([]);
	});
}

test("cold navigation keeps the current page visible while loading the next module", async ({
	page,
}) => {
	await mockPageApi(page);
	await page.goto(`${guildRoot}/audio`);
	const audioPage = page
		.getByRole("button", { name: "Browse files" })
		.or(page.getByRole("textbox", { name: "Search recordings" }));
	await expect(audioPage).toBeVisible();
	const moduleGate = gate();
	await page.route(modulePattern("clips"), async (route) => {
		await moduleGate.promise;
		await route.continue();
	});
	try {
		const dataReady = page.waitForResponse(
			(response) => new URL(response.url()).pathname === clipsPath,
		);
		await page.getByRole("button", { name: "Clips", exact: true }).click();
		await (await dataReady).finished();
		await expect(audioPage).toBeVisible();
		await expect(page.getByText("Loading Route", { exact: true })).toHaveCount(
			0,
		);
		moduleGate.release();
		await expect(page).toHaveURL(new RegExp(`${guildRoot}/clips$`));
	} finally {
		moduleGate.release();
	}
});

test("clips refresh on role changes and return visits, without refreshing on selection", async ({
	page,
}) => {
	await mockPageApi(page);
	const requests: string[] = [];
	page.on("request", (request) => {
		const url = new URL(request.url());
		if (url.pathname === clipsPath) requests.push(url.search);
	});
	await page.goto(`${guildRoot}/clips?as_role=role-123`);
	await page.getByRole("button", { name: "Exit preview" }).click();
	await expect(page).toHaveURL(new RegExp(`${guildRoot}/clips$`));
	const browse = page.getByRole("button", { name: "Browse clips" });
	await expect(
		browse.or(page.getByRole("textbox", { name: "Search clips" })),
	).toBeVisible();
	if (await browse.isVisible()) await browse.click();
	await page.getByRole("button", { name: /^Working source/ }).click();
	await expect(page).toHaveURL(/\/clips\/working-source$/);
	expect(requests).toEqual(["?as_role=role-123", ""]);
	const openNavigation = page.getByRole("button", { name: "open navigation" });
	if (await openNavigation.isVisible()) await openNavigation.click();
	await page.getByRole("button", { name: "Audio", exact: true }).click();
	await expect(page).toHaveURL(/\/audio$/);
	await page.goBack();
	await expect(page).toHaveURL(/\/clips\/working-source$/);
	await expect.poll(() => requests).toEqual(["?as_role=role-123", "", ""]);
});

test("a failed members module offers reload and server navigation", async ({
	page,
	consoleAudit,
}) => {
	await mockPageApi(page);
	consoleAudit.allow(
		/net::ERR_FAILED|Failed to fetch dynamically imported module|Importing a module script failed|error occurred in one of your React components/i,
	);
	await page.route(modulePattern("members"), (route) => route.abort("failed"));
	await page.goto(`${guildRoot}/members`);
	await expect(
		page.getByRole("heading", { name: "Could not load the members page" }),
	).toBeVisible();
	await expect(
		page.getByRole("button", { name: "Reload", exact: true }),
	).toBeVisible();
	await page.getByRole("link", { name: "Choose a server" }).click();
	await expect(
		page.getByRole("heading", { name: "Choose a server" }),
	).toBeVisible();
});

test("stamps without a selected guild makes no stamps request", async ({
	page,
}) => {
	await mockPageApi(page);
	const requests: string[] = [];
	page.on("request", (request) => {
		if (new URL(request.url()).pathname.startsWith("/api/stamps/"))
			requests.push(request.url());
	});
	await page.goto("/stamps");
	await expect(
		page.getByText("Select a guild from the top navbar to view stamps."),
	).toBeVisible();
	expect(requests).toEqual([]);
});

test("voice settings renders its page while the data response is pending", async ({
	page,
}) => {
	await mockPageApi(page);
	const dataGate = gate();
	await page.route(
		`${API_ORIGIN}${adminRoot}/voice-settings`,
		async (route) => {
			await dataGate.promise;
			await route.fallback();
		},
	);
	try {
		await page.goto(`${guildRoot}/admin/voice-settings`);
		await expect(
			page.getByRole("heading", { name: "Voice Settings", exact: true }),
		).toBeVisible();
		await expect(page.getByText("Loading Route", { exact: true })).toHaveCount(
			0,
		);
	} finally {
		dataGate.release();
	}
});
