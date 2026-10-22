import { describe, expect, test } from "bun:test";
import type {
	CapabilityDecision,
	PermissionRule,
	PermissionRules,
} from "../../app/apiSlice";
import {
	describeSource,
	EVERYONE,
	isEmpty,
	NO_EFFECT,
	ruleFor,
	sameEffect,
	stateOf,
	subjectName,
	withState,
} from "./rules";

const data: Pick<PermissionRules, "roles" | "rules" | "channels"> = {
	roles: [
		{
			role_id: "11",
			name: "VIP",
			color: 0,
			color_secondary: null,
			color_tertiary: null,
		},
	],
	channels: [{ channel_id: "100", name: "lounge", editable: [] }],
	rules: [
		{
			channel_id: "100",
			subject: { kind: "user", id: "7" },
			name: "Ada",
			allow: ["jam"],
			deny: [],
			reaches_nobody: false,
			updated_by: "1",
			updated_at: "2026-10-22T00:00:00Z",
		} satisfies PermissionRule,
	],
};

describe("rule effects", () => {
	test("each capability is allowed, denied or left to the layers below", () => {
		let effect = withState(NO_EFFECT, "stamp", "deny");
		effect = withState(effect, "jam", "allow");
		effect = withState(effect, "manage_settings", "allow");
		expect(effect).toEqual({
			allow: ["manage_settings", "jam"],
			deny: ["stamp"],
		});
		expect(stateOf(effect, "stamp")).toBe("deny");
		expect(stateOf(effect, "jam")).toBe("allow");
		expect(stateOf(effect, "listen_others")).toBe("inherit");

		// Never both at once.
		const flipped = withState(effect, "stamp", "allow");
		expect(flipped.deny).toEqual([]);
		expect(flipped.allow).toContain("stamp");
		const cleared = withState(
			withState(withState(flipped, "stamp", "inherit"), "jam", "inherit"),
			"manage_settings",
			"inherit",
		);
		expect(isEmpty(cleared)).toBe(true);
		expect(
			sameEffect(effect, {
				allow: ["jam", "manage_settings"],
				deny: ["stamp"],
			}),
		).toBe(true);
		expect(sameEffect(effect, cleared)).toBe(false);
	});
});

describe("subjects", () => {
	test("are named after their role or member, or what is left of them", () => {
		expect(subjectName(EVERYONE, data)).toBe("@everyone");
		expect(subjectName({ kind: "role", id: "11" }, data)).toBe("@VIP");
		expect(subjectName({ kind: "role", id: "12" }, data)).toBe(
			"Deleted role 12",
		);
		expect(subjectName({ kind: "user", id: "7" }, data)).toBe("Ada");
		expect(subjectName({ kind: "user", id: "8" }, data, "Grace")).toBe("Grace");
		expect(subjectName({ kind: "user", id: "8" }, data)).toBe("User 8");
	});

	test("find their rule at one scope only", () => {
		const subject = { kind: "user" as const, id: "7" };
		expect(ruleFor(data.rules, "100", subject)?.name).toBe("Ada");
		expect(ruleFor(data.rules, null, subject)).toBeUndefined();
		expect(ruleFor(data.rules, "100", EVERYONE)).toBeUndefined();
	});
});

describe("describeSource", () => {
	const decision = (
		source: CapabilityDecision["source"],
		held = true,
	): CapabilityDecision => ({
		capability: "jam",
		held,
		discord: !held,
		source,
	});

	test("names the layer that decided", () => {
		expect(
			describeSource(
				decision({ kind: "discord", channel_id: null, subject: null }),
				data,
			),
		).toBe("Discord's permissions");
		expect(
			describeSource(
				decision({ kind: "owner", channel_id: null, subject: null }),
				data,
			),
		).toBe("The owner always keeps it");
		expect(
			describeSource(
				decision({
					kind: "rule",
					channel_id: "100",
					subject: { kind: "user", id: "7" },
				}),
				data,
			),
		).toBe("Allowed for Ada in lounge");
		expect(
			describeSource(
				decision(
					{
						kind: "rule",
						channel_id: null,
						subject: { kind: "role", id: "11" },
					},
					false,
				),
				data,
			),
		).toBe("Denied for @VIP server-wide");
	});
});
