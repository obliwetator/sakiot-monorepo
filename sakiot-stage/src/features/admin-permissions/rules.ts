import type {
	CapabilityDecision,
	PermissionRule,
	PermissionRules,
	RuleSubject,
} from "../../app/apiSlice";
import type { Capability } from "../../shared/permissions";

// The Discord permissions each capability follows when no rule decides, as
// `Capabilities::from_guild_permissions` and `from_channel_permissions` map
// them in sakiot-permissions.
const MANAGER = "Manage Server or Administrator";
const VIEWER = "View Channel";
const LISTENER = "View Channel and Connect";

/**
 * What each capability is called here, what it covers, and the Discord
 * permissions that decide it when no rule does.
 */
export const CAPABILITIES: Record<
	Capability,
	{ label: string; description: string; discord: string }
> = {
	manage_settings: {
		label: "Manage settings",
		description: "Jam cooldowns, the recording policy and voice settings.",
		discord: MANAGER,
	},
	manage_permissions: {
		label: "Edit permissions",
		description:
			"These rules, within what they hold themselves. No rule takes anything from them in a channel, though one may give them more.",
		discord: MANAGER,
	},
	preview_roles: {
		label: "Preview roles",
		description: "The Members page, and viewing the server as a role.",
		discord: MANAGER,
	},
	delete_recordings: {
		label: "Delete recordings",
		description:
			"Remove recordings from view, and see private ones as redacted entries.",
		discord: MANAGER,
	},
	moderate_clips: {
		label: "Moderate clips",
		description: "Rename, delete or overwrite other members' clips.",
		discord: MANAGER,
	},
	view_presence: {
		label: "See who is in voice",
		description:
			"The voice presence panel. Without it, nothing else in the channel is held, as with Discord's View Channel.",
		discord: VIEWER,
	},
	listen_others: {
		label: "Listen to others",
		description:
			"Other members' recordings, live audio, waveforms, downloads and mixes. Never anyone who keeps their recordings private.",
		discord: LISTENER,
	},
	create_clips: {
		label: "Make clips",
		description: "Clip and compose recordings they can hear.",
		discord: LISTENER,
	},
	jam: {
		label: "Play clips in voice",
		description:
			"/jam, and playing clips from the dashboard, where the bot is.",
		discord: LISTENER,
	},
	stamp: {
		label: "Stamp",
		description: "/stamp, in the voice channel they are in.",
		discord: VIEWER,
	},
};

/** Server-wide only. */
export const MANAGEMENT: Capability[] = [
	"manage_settings",
	"manage_permissions",
	"preview_roles",
	"delete_recordings",
	"moderate_clips",
];

/** Per voice channel; a server-wide rule sets them in every channel. */
export const CONTENT: Capability[] = [
	"view_presence",
	"listen_others",
	"create_clips",
	"jam",
	"stamp",
];

/** What a rule does with one capability. */
export type RuleState = "allow" | "inherit" | "deny";

export interface RuleEffect {
	allow: Capability[];
	deny: Capability[];
}

export const NO_EFFECT: RuleEffect = { allow: [], deny: [] };

export function stateOf(effect: RuleEffect, capability: Capability): RuleState {
	if (effect.allow.includes(capability)) return "allow";
	if (effect.deny.includes(capability)) return "deny";
	return "inherit";
}

/** `effect` with `capability` set to `state`, in the canonical order. */
export function withState(
	effect: RuleEffect,
	capability: Capability,
	state: RuleState,
): RuleEffect {
	const order = [...MANAGEMENT, ...CONTENT];
	const without = (list: Capability[]) =>
		list.filter((held) => held !== capability);
	const sorted = (list: Capability[]) =>
		order.filter((known) => list.includes(known));
	return {
		allow: sorted(
			state === "allow"
				? [...without(effect.allow), capability]
				: without(effect.allow),
		),
		deny: sorted(
			state === "deny"
				? [...without(effect.deny), capability]
				: without(effect.deny),
		),
	};
}

export function sameEffect(a: RuleEffect, b: RuleEffect): boolean {
	const same = (x: Capability[], y: Capability[]) =>
		x.length === y.length && x.every((capability) => y.includes(capability));
	return same(a.allow, b.allow) && same(a.deny, b.deny);
}

export function isEmpty(effect: RuleEffect): boolean {
	return effect.allow.length === 0 && effect.deny.length === 0;
}

/** One key per subject, for lists and selection. */
export function subjectKey(subject: RuleSubject): string {
	return subject.kind === "everyone"
		? "everyone"
		: `${subject.kind}:${subject.id ?? ""}`;
}

export const EVERYONE: RuleSubject = { kind: "everyone" };

/** Whom a subject names, as the editor shows it. */
export function subjectName(
	subject: RuleSubject,
	data: Pick<PermissionRules, "roles" | "rules">,
	known?: string,
): string {
	if (subject.kind === "everyone") return "@everyone";
	if (subject.kind === "role") {
		const role = data.roles.find((role) => role.role_id === subject.id);
		return role ? `@${role.name}` : `Deleted role ${subject.id ?? ""}`;
	}
	const named = data.rules.find(
		(rule) => rule.subject.kind === "user" && rule.subject.id === subject.id,
	)?.name;
	return known ?? named ?? `User ${subject.id ?? ""}`;
}

/** The rule at `channel` (null: server-wide) for `subject`, if any. */
export function ruleFor(
	rules: PermissionRule[],
	channel: string | null,
	subject: RuleSubject,
): PermissionRule | undefined {
	const key = subjectKey(subject);
	return rules.find(
		(rule) =>
			(rule.channel_id ?? null) === channel && subjectKey(rule.subject) === key,
	);
}

/**
 * What decided a capability, in words: Discord's permissions, the owner's
 * standing, not seeing the channel, the viewer's own access (in a role
 * preview), a rule that takes nothing from an editor, or a rule and where
 * it applies.
 */
export function describeSource(
	decision: CapabilityDecision,
	data: Pick<PermissionRules, "roles" | "rules" | "channels">,
): string {
	const { source } = decision;
	if (source.kind === "owner") return "The owner always keeps it";
	if (source.kind === "unseen") return "Taken by not seeing the channel";
	if (source.kind === "manager")
		return "Left out of the preview: you don't hold it yourself";
	if (source.kind === "exempt")
		return "Kept: rules take nothing from whoever can edit them";
	if (source.kind === "discord" || !source.subject)
		return "Discord's permissions";
	const whom = subjectName(source.subject, data);
	const verb = decision.held ? "Allowed" : "Denied";
	if (!source.channel_id) return `${verb} for ${whom} server-wide`;
	const channel = data.channels.find(
		(channel) => channel.channel_id === source.channel_id,
	);
	return `${verb} for ${whom} in ${channel ? channel.name : "this channel"}`;
}
