import { Trash2 } from "lucide-react";
import { useState } from "react";
import { problemFromError } from "../../app/apiError";
import {
	type PermissionRules,
	type RuleSubject,
	useSetPermissionRuleMutation,
} from "../../app/apiSlice";
import { MemberPicker } from "../../shared/MemberPicker";
import type { Capability } from "../../shared/permissions";
import {
	Button,
	cn,
	IconButton,
	Notice,
	Radio,
	RadioGroup,
	Select,
	SelectItem,
	Text,
} from "../../shared/ui";
import { roleSwatchBackground } from "../members/roleColors";
import {
	CAPABILITIES,
	CONTENT,
	EVERYONE,
	isEmpty,
	MANAGEMENT,
	NO_EFFECT,
	type RuleEffect,
	type RuleState,
	ruleFor,
	sameEffect,
	stateOf,
	subjectKey,
	subjectName,
	withState,
} from "./rules";

const SERVER = "server";

/** What a refused change says, from the server when it explains why. */
export function ruleFailure(error: unknown, context: string): string {
	const problem = problemFromError(error);
	if (
		problem.status === 403 ||
		problem.kind === "invalid_request" ||
		problem.kind === "not_found"
	)
		return problem.message;
	return `${context} ${problem.message}`;
}

interface Picked {
	subject: RuleSubject;
	/** How the member picker named a member without a rule yet. */
	name?: string;
}

export function RuleEditor({
	guildId,
	data,
}: {
	guildId: string;
	data: PermissionRules;
}) {
	const [scope, setScope] = useState<string>(SERVER);
	const [picked, setPicked] = useState<Picked[]>([]);
	const [selected, setSelected] = useState<string>(subjectKey(EVERYONE));
	const channel = scope === SERVER ? null : scope;
	const editable: Capability[] =
		channel === null
			? data.editable
			: (data.channels.find((c) => c.channel_id === channel)?.editable ?? []);

	const subjects: Picked[] = [{ subject: EVERYONE }];
	const add = (entry: Picked) => {
		if (
			!subjects.some((s) => subjectKey(s.subject) === subjectKey(entry.subject))
		)
			subjects.push(entry);
	};
	for (const rule of data.rules)
		if ((rule.channel_id ?? null) === channel && !rule.reaches_nobody)
			add({ subject: rule.subject });
	for (const entry of picked) add(entry);
	const current: Picked = subjects.find(
		(entry) => subjectKey(entry.subject) === selected,
	) ?? { subject: EVERYONE };

	const pick = (entry: Picked) => {
		setPicked((list) => [...list, entry]);
		setSelected(subjectKey(entry.subject));
	};
	const unlistedRoles = data.roles.filter(
		(role) =>
			!subjects.some(
				(entry) =>
					entry.subject.kind === "role" && entry.subject.id === role.role_id,
			),
	);

	return (
		<div className="space-y-4">
			<Select
				label="Where"
				selectedKey={scope}
				onSelectionChange={(key) => {
					setScope(String(key));
					setSelected(subjectKey(EVERYONE));
				}}
				className="sm:w-72"
			>
				<SelectItem id={SERVER}>Whole server</SelectItem>
				{data.channels.map((channel) => (
					<SelectItem key={channel.channel_id} id={channel.channel_id}>
						{channel.name || channel.channel_id}
					</SelectItem>
				))}
			</Select>

			<div className="flex flex-col gap-4 min-[900px]:flex-row">
				<div className="space-y-3 min-[900px]:w-72 min-[900px]:shrink-0">
					<ul aria-label="Rules here" className="flex flex-col gap-1">
						{subjects.map((entry) => {
							const key = subjectKey(entry.subject);
							const role =
								entry.subject.kind === "role"
									? data.roles.find((r) => r.role_id === entry.subject.id)
									: undefined;
							const rule = ruleFor(data.rules, channel, entry.subject);
							return (
								<li key={key}>
									<Button
										variant="ghost"
										aria-pressed={key === subjectKey(current.subject)}
										onPress={() => setSelected(key)}
										className="w-full justify-start text-left aria-pressed:border-accent/50 aria-pressed:bg-accent/10"
									>
										{role && (
											<span
												aria-hidden="true"
												className="mr-2 size-3 shrink-0 rounded-sm"
												style={{ background: roleSwatchBackground(role) }}
											/>
										)}
										<span className="truncate">
											{subjectName(entry.subject, data, entry.name)}
										</span>
										{!rule && (
											<span className="ml-auto text-xs text-muted">
												no rule
											</span>
										)}
									</Button>
								</li>
							);
						})}
					</ul>
					{unlistedRoles.length > 0 && (
						<Select
							label="Add a role"
							selectedKey={null}
							onSelectionChange={(key) => {
								if (key !== null)
									pick({ subject: { kind: "role", id: String(key) } });
							}}
						>
							{unlistedRoles.map((role) => (
								<SelectItem key={role.role_id} id={role.role_id}>
									{role.name}
								</SelectItem>
							))}
						</Select>
					)}
					<MemberPicker
						guildId={guildId}
						label="Add a member"
						onPick={(userId, name) =>
							pick({ subject: { kind: "user", id: userId }, name })
						}
					/>
				</div>

				<RuleForm
					key={`${scope}|${subjectKey(current.subject)}`}
					guildId={guildId}
					channel={channel}
					subject={current.subject}
					name={subjectName(current.subject, data, current.name)}
					saved={ruleFor(data.rules, channel, current.subject) ?? NO_EFFECT}
					editable={editable}
				/>
			</div>
		</div>
	);
}

const STATES: { state: RuleState; label: string }[] = [
	{ state: "deny", label: "Deny" },
	{ state: "inherit", label: "Default" },
	{ state: "allow", label: "Allow" },
];

function RuleForm({
	guildId,
	channel,
	subject,
	name,
	saved,
	editable,
}: {
	guildId: string;
	channel: string | null;
	subject: RuleSubject;
	name: string;
	saved: RuleEffect;
	editable: Capability[];
}) {
	const [draft, setDraft] = useState<RuleEffect | null>(null);
	const [setRule, state] = useSetPermissionRuleMutation();
	const effect = draft ?? saved;
	const dirty = draft !== null && !sameEffect(draft, saved);
	const capabilities = channel === null ? [...MANAGEMENT, ...CONTENT] : CONTENT;

	const save = async (next: RuleEffect) => {
		state.reset();
		try {
			await setRule({
				guild_id: guildId,
				channel_id: channel,
				subject,
				allow: next.allow,
				deny: next.deny,
			}).unwrap();
			setDraft(null);
		} catch {
			// The failure shows below; the draft stays to retry or change.
		}
	};

	return (
		<section
			aria-label={`Rule for ${name}`}
			className="min-w-0 flex-1 space-y-3"
		>
			<Text tone="muted">
				{channel === null
					? "Server-wide: applies in every voice channel unless a channel's own rule says otherwise."
					: "In this channel: overrides every server-wide rule here."}
			</Text>
			<ul className="divide-y divide-ui-border rounded-lg border border-ui-border">
				{capabilities.map((capability) => {
					const { label, description } = CAPABILITIES[capability];
					const locked = !editable.includes(capability);
					return (
						<li
							key={capability}
							className="flex flex-col gap-2 p-3 sm:flex-row sm:items-center"
						>
							<div className="min-w-0 flex-1">
								<p className="text-sm font-medium text-fg">{label}</p>
								<p className="text-xs text-muted">
									{description}
									{locked &&
										" You don't hold this here, so you can't change it."}
								</p>
							</div>
							<RadioGroup
								aria-label={`${label} for ${name}`}
								orientation="horizontal"
								value={stateOf(effect, capability)}
								isDisabled={locked}
								onChange={(value) =>
									setDraft(withState(effect, capability, value as RuleState))
								}
								className="flex-row gap-3"
							>
								{STATES.map(({ state, label }) => (
									<Radio
										key={state}
										value={state}
										className={cn(
											state === "allow" && "data-[selected]:text-emerald-300",
											state === "deny" && "data-[selected]:text-rose-300",
										)}
									>
										{label}
									</Radio>
								))}
							</RadioGroup>
						</li>
					);
				})}
			</ul>
			<div className="flex flex-wrap items-center gap-2">
				<Button
					variant="primary"
					isDisabled={!dirty}
					isPending={state.isLoading}
					onPress={() => void save(effect)}
				>
					Save rule
				</Button>
				<Button
					variant="ghost"
					isDisabled={!dirty || state.isLoading}
					onPress={() => {
						setDraft(null);
						state.reset();
					}}
				>
					Discard changes
				</Button>
				{!isEmpty(saved) && (
					<IconButton
						variant="danger"
						aria-label={`Remove the rule for ${name}`}
						isDisabled={state.isLoading}
						onPress={() => void save(NO_EFFECT)}
					>
						<Trash2 aria-hidden="true" className="size-4" />
					</IconButton>
				)}
			</div>
			{state.isSuccess && (
				<Notice tone="success" announce="status">
					Rule saved.
				</Notice>
			)}
			{state.isError && (
				<Notice tone="error" announce="alert">
					{ruleFailure(state.error, "Could not save the rule.")}
				</Notice>
			)}
		</section>
	);
}

/** Rules whose role or channel is gone: they reach nobody, so only go. */
export function StaleRules({
	guildId,
	data,
}: {
	guildId: string;
	data: PermissionRules;
}) {
	const [setRule, state] = useSetPermissionRuleMutation();
	const stale = data.rules.filter((rule) => rule.reaches_nobody);
	if (stale.length === 0) return null;
	return (
		<div className="space-y-2">
			<Text tone="muted">
				These rules reach nobody: their role or channel is gone.
			</Text>
			<ul className="flex flex-col gap-1">
				{stale.map((rule) => {
					const where = rule.channel_id
						? `channel ${rule.channel_id}`
						: "server-wide";
					const whom = subjectName(rule.subject, data, rule.name ?? undefined);
					return (
						<li
							key={`${rule.channel_id ?? SERVER}|${subjectKey(rule.subject)}`}
							className="flex items-center gap-2 text-sm"
						>
							<span className="flex-1">
								{whom}, {where}
							</span>
							<IconButton
								variant="danger"
								aria-label={`Delete the rule for ${whom}, ${where}`}
								isDisabled={state.isLoading}
								onPress={() =>
									void setRule({
										guild_id: guildId,
										channel_id: rule.channel_id ?? null,
										subject: rule.subject,
										allow: [],
										deny: [],
									})
								}
							>
								<Trash2 aria-hidden="true" className="size-4" />
							</IconButton>
						</li>
					);
				})}
			</ul>
			{state.isError && (
				<Notice tone="error" announce="alert">
					{ruleFailure(state.error, "Could not delete the rule.")}
				</Notice>
			)}
		</div>
	);
}
