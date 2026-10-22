import { Check, X } from "lucide-react";
import { useState } from "react";
import {
	type CapabilityDecision,
	type PermissionRules,
	useExplainAccessQuery,
} from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import { MemberPicker } from "../../shared/MemberPicker";
import {
	cn,
	Select,
	SelectItem,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Text,
} from "../../shared/ui";
import { CAPABILITIES, CONTENT, describeSource } from "./rules";

type Target =
	| { kind: "role"; id: string; name: string }
	| { kind: "user"; id: string; name: string };

const MEMBERSHIP: Record<string, string> = {
	owner: "Owner of the server.",
	member: "Member of the server.",
	former:
		"No longer a member: they reach only their own recordings, and no rule changes that.",
	outsider: "Not a member: they reach nothing, and no rule changes that.",
};

/** What one member or role holds, capability by capability, and why. */
export function AccessExplainer({
	guildId,
	data,
	everyoneId,
}: {
	guildId: string;
	data: PermissionRules;
	/** The server's id, which is `@everyone`'s. */
	everyoneId: string;
}) {
	const [target, setTarget] = useState<Target | null>(null);
	const query = useExplainAccessQuery(
		{
			guild_id: guildId,
			user_id: target?.kind === "user" ? target.id : undefined,
			role_id: target?.kind === "role" ? target.id : undefined,
		},
		{ skip: target === null },
	);
	const roles = [
		{ role_id: everyoneId, name: "@everyone" },
		...data.roles.map((role) => ({ ...role, name: `@${role.name}` })),
	];
	const explained = query.currentData;

	return (
		<div className="space-y-4">
			<div className="grid gap-3 sm:grid-cols-2">
				<Select
					label="A role"
					selectedKey={target?.kind === "role" ? target.id : null}
					onSelectionChange={(key) => {
						const role = roles.find((role) => role.role_id === key);
						if (role)
							setTarget({ kind: "role", id: role.role_id, name: role.name });
					}}
				>
					{roles.map((role) => (
						<SelectItem key={role.role_id} id={role.role_id}>
							{role.name}
						</SelectItem>
					))}
				</Select>
				<MemberPicker
					guildId={guildId}
					label="Or a member"
					onPick={(id, name) => setTarget({ kind: "user", id, name })}
				/>
			</div>

			{target && query.isFetching && !explained && (
				<Text aria-live="polite">Loading {target.name}'s access…</Text>
			)}
			{query.isError && (
				<LoadFailure
					error={query.error}
					what="Could not explain this access."
					onRetry={() => void query.refetch()}
				/>
			)}
			{target && explained && (
				<section aria-label={`Access of ${target.name}`} className="space-y-3">
					<Text>
						<strong>{target.name}</strong>
						{target.kind === "role"
							? " — a member holding only this role."
							: ` — ${MEMBERSHIP[explained.membership]}`}
					</Text>
					{explained.guild.length > 0 && (
						<ul className="flex flex-wrap gap-2" aria-label="Server-wide">
							{explained.guild.map((decision) => (
								<li
									key={decision.capability}
									className="flex items-center gap-1.5 rounded-md border border-ui-border px-2 py-1 text-sm"
									title={describeSource(decision, data)}
								>
									<Held decision={decision} />
									{CAPABILITIES[decision.capability].label}
									<Source decision={decision} data={data} />
								</li>
							))}
						</ul>
					)}
					{explained.channels.length > 0 && (
						// Wider than a phone: the region scrolls, so keyboards must
						// reach it.
						<section
							aria-label={`${target.name} in each voice channel`}
							// biome-ignore lint/a11y/noNoninteractiveTabindex: a scrolling region must be focusable to scroll by keyboard.
							tabIndex={0}
							className="w-full overflow-x-auto rounded-lg border border-ui-border focus-visible:outline-2 focus-visible:outline-focus"
						>
							<Table>
								<caption className="sr-only">
									What {target.name} holds in each voice channel
								</caption>
								<TableHeader>
									<TableRow>
										<TableHead scope="col">Channel</TableHead>
										{CONTENT.map((capability) => (
											<TableHead key={capability} scope="col">
												{CAPABILITIES[capability].label}
											</TableHead>
										))}
									</TableRow>
								</TableHeader>
								<TableBody>
									{explained.channels.map((channel) => (
										<TableRow key={channel.channel_id}>
											<TableCell className="font-medium">
												{channel.name || channel.channel_id}
											</TableCell>
											{channel.capabilities.map((decision) => (
												<TableCell
													key={decision.capability}
													title={describeSource(decision, data)}
												>
													<span className="flex items-center gap-1.5">
														<Held decision={decision} />
														<Source decision={decision} data={data} />
													</span>
												</TableCell>
											))}
										</TableRow>
									))}
								</TableBody>
							</Table>
						</section>
					)}
					<Text tone="muted" className="text-xs">
						Hover a capability to see what decided it. Listening also needs each
						speaker's consent: nobody hears a member who keeps their recordings
						private.
					</Text>
				</section>
			)}
		</div>
	);
}

function Held({ decision }: { decision: CapabilityDecision }) {
	return decision.held ? (
		<>
			<Check aria-hidden="true" className="size-4 text-emerald-300" />
			<span className="sr-only">Allowed</span>
		</>
	) : (
		<>
			<X aria-hidden="true" className="size-4 text-rose-300" />
			<span className="sr-only">Denied</span>
		</>
	);
}

/** A short mark when a rule or the owner's standing decided, not Discord. */
function Source({
	decision,
	data,
}: {
	decision: CapabilityDecision;
	data: PermissionRules;
}) {
	if (decision.source.kind === "discord") return null;
	return (
		<span
			className={cn(
				"rounded px-1 text-[0.65rem] uppercase tracking-wide",
				decision.source.kind === "owner"
					? "bg-slate-700 text-slate-200"
					: "bg-accent/20 text-accent",
			)}
		>
			<span aria-hidden="true">
				{decision.source.kind === "owner" ? "owner" : "rule"}
			</span>
			<span className="sr-only">{describeSource(decision, data)}</span>
		</span>
	);
}
