import type { CapabilityDecision, GuildRole } from "../../app/apiSlice";
import type { Capability } from "../../shared/permissions";
import { CAPABILITIES } from "../admin-permissions/rules";

/** What a previewed role does in one voice channel, at a glance. */
export type ChannelStatus = "listens" | "sees" | "hidden";

export function channelStatus(decisions: CapabilityDecision[]): ChannelStatus {
	const held = (capability: Capability) =>
		decisions.some(
			(decision) => decision.capability === capability && decision.held,
		);
	if (held("listen_others")) return "listens";
	if (held("view_presence")) return "sees";
	return "hidden";
}

/** One sentence on why the role holds or lacks something. */
export interface Reason {
	held: boolean;
	text: string;
}

/**
 * Why the role holds or lacks what Discord alone does not decide, at one
 * scope (`channel` null: server-wide): one sentence per Sakiot rule's
 * decision, then one for what not seeing the channel takes, then one for
 * what the preview leaves out because the viewer lacks it.
 */
export function reasons(
	decisions: CapabilityDecision[],
	roles: Pick<GuildRole, "role_id" | "name">[],
	channel: string | null,
): Reason[] {
	const ruled = decisions
		.filter((decision) => decision.source.kind === "rule")
		.map((decision) => ({
			held: decision.held,
			text: ruleSentence(decision, roles, channel),
		}));
	const of = (kind: CapabilityDecision["source"]["kind"]) =>
		decisions
			.filter((decision) => decision.source.kind === kind)
			.map((decision) => CAPABILITIES[decision.capability].label);
	const unseen = of("unseen");
	const beyond = of("manager");
	return [
		...ruled,
		...(unseen.length > 0
			? [
					{
						held: false,
						text: `${listed(unseen)}: not allowed, because the role can't see this channel.`,
					},
				]
			: []),
		...(beyond.length > 0
			? [
					{
						held: false,
						text: `${listed(beyond)}: left out, because you don't hold ${beyond.length === 1 ? "it" : "them"}${channel === null ? "" : " here"} yourself.`,
					},
				]
			: []),
	];
}

function ruleSentence(
	decision: CapabilityDecision,
	roles: Pick<GuildRole, "role_id" | "name">[],
	channel: string | null,
): string {
	const { label } = CAPABILITIES[decision.capability];
	const { subject, channel_id } = decision.source;
	const whom =
		subject?.kind === "role"
			? roleName(roles, subject.id)
			: subject?.kind === "user"
				? "a member"
				: "@everyone";
	const where =
		channel_id === null
			? "server-wide"
			: channel_id === channel
				? "in this channel"
				: "in another channel";
	const verb = decision.held ? "allowed" : "denied";
	const discord =
		decision.held === decision.discord
			? ""
			: decision.held
				? " Discord alone wouldn't allow it."
				: " Discord alone would allow it.";
	return `${label}: ${verb} by a Sakiot rule for ${whom} ${where}.${discord}`;
}

function roleName(
	roles: Pick<GuildRole, "role_id" | "name">[],
	id: string | null | undefined,
): string {
	const name = roles.find((role) => role.role_id === id)?.name;
	if (!name) return "a deleted role";
	return name.startsWith("@") ? name : `@${name}`;
}

/** "A", "A and B", "A, B and C". */
function listed(labels: string[]): string {
	if (labels.length <= 1) return labels.join("");
	return `${labels.slice(0, -1).join(", ")} and ${labels.at(-1)}`;
}
