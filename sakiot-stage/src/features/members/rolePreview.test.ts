import { describe, expect, test } from "bun:test";
import type { CapabilityDecision } from "../../app/apiSlice";
import type { Capability } from "../../shared/permissions";
import { channelStatus, reasons } from "./rolePreview";

const GENERAL = "500";
const roles = [
	{ role_id: "1", name: "@everyone" },
	{ role_id: "11", name: "VIP" },
];

const decision = (
	capability: Capability,
	held: boolean,
	source: Partial<CapabilityDecision["source"]> & {
		kind: CapabilityDecision["source"]["kind"];
	},
	discord = true,
): CapabilityDecision => ({
	capability,
	held,
	discord,
	source: { channel_id: null, subject: null, ...source },
});

/** @everyone denied seeing General: everything else there goes with it. */
const hiddenGeneral: CapabilityDecision[] = [
	decision("view_presence", false, {
		kind: "rule",
		channel_id: GENERAL,
		subject: { kind: "everyone" },
	}),
	decision("listen_others", false, { kind: "unseen" }),
	decision("create_clips", false, { kind: "unseen" }),
	decision("jam", false, { kind: "unseen" }),
	decision("stamp", false, { kind: "unseen" }),
];

describe("channelStatus", () => {
	test("listening first, then seeing", () => {
		expect(channelStatus(hiddenGeneral)).toBe("hidden");
		expect(
			channelStatus([
				decision("view_presence", true, { kind: "discord" }),
				decision("listen_others", false, { kind: "discord" }, false),
			]),
		).toBe("sees");
		expect(
			channelStatus([
				decision("view_presence", true, { kind: "discord" }),
				decision("listen_others", true, { kind: "discord" }),
			]),
		).toBe("listens");
	});
});

describe("reasons", () => {
	test("say a Sakiot rule hid the channel, and what that took", () => {
		expect(reasons(hiddenGeneral, roles, GENERAL)).toEqual([
			{
				held: false,
				text: "See who is in voice: denied by a Sakiot rule for @everyone in this channel. Discord alone would allow it.",
			},
			{
				held: false,
				text: "Listen to others, Make clips, Play clips in voice and Stamp: not allowed, because the role can't see this channel.",
			},
		]);
	});

	test("name the role, the scope, and whether Discord agrees", () => {
		expect(
			reasons(
				[
					decision(
						"jam",
						true,
						{ kind: "rule", subject: { kind: "role", id: "11" } },
						false,
					),
					decision("stamp", false, {
						kind: "rule",
						subject: { kind: "role", id: "99" },
					}),
					decision("create_clips", true, { kind: "rule" }),
				],
				roles,
				GENERAL,
			).map((reason) => reason.text),
		).toEqual([
			"Play clips in voice: allowed by a Sakiot rule for @VIP server-wide. Discord alone wouldn't allow it.",
			"Stamp: denied by a Sakiot rule for a deleted role server-wide. Discord alone would allow it.",
			"Make clips: allowed by a Sakiot rule for @everyone server-wide.",
		]);
	});

	test("say what the preview leaves out because you lack it", () => {
		expect(
			reasons(
				[
					decision("moderate_clips", false, { kind: "manager" }),
					decision("delete_recordings", false, { kind: "manager" }),
				],
				roles,
				null,
			),
		).toEqual([
			{
				held: false,
				text: "Moderate clips and Delete recordings: left out, because you don't hold them yourself.",
			},
		]);
		expect(
			reasons([decision("jam", false, { kind: "manager" })], roles, GENERAL)[0]
				?.text,
		).toBe(
			"Play clips in voice: left out, because you don't hold it here yourself.",
		);
	});

	test("say what a role that can edit permissions keeps", () => {
		expect(
			reasons(
				[
					decision("view_presence", true, { kind: "exempt" }),
					decision("listen_others", true, { kind: "exempt" }),
				],
				roles,
				GENERAL,
			),
		).toEqual([
			{
				held: true,
				text: "See who is in voice and Listen to others: kept, because this role can edit permissions and no Sakiot rule takes anything from it.",
			},
		]);
	});

	test("say nothing where Discord decided", () => {
		expect(
			reasons(
				[decision("listen_others", true, { kind: "discord" })],
				roles,
				GENERAL,
			),
		).toEqual([]);
	});
});
