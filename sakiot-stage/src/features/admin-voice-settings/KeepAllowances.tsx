import { Trash2 } from "lucide-react";
import { useState } from "react";
import { managerActionFailure as failure } from "../../app/apiError";
import {
	type KeepAllowances,
	type RuleSubject,
	useGetKeepAllowancesQuery,
	useSetKeepAllowanceMutation,
} from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import { MemberPicker } from "../../shared/MemberPicker";
import {
	Button,
	IconButton,
	Notice,
	Select,
	SelectItem,
	TextField,
} from "../../shared/ui";
import { EVERYONE, subjectKey } from "../admin-permissions/rules";
import { recordings } from "../kept-recordings/keeping";

/** The server's `MAX_KEPT`. */
const MAX_ALLOWANCE = 100_000;

interface Listed {
	subject: RuleSubject;
	name: string;
	saved: number | null;
	reachesNobody: boolean;
}

/** Whom an allowance is for, as the panel names it. */
function allowanceName(
	subject: RuleSubject,
	data: Pick<KeepAllowances, "roles">,
	known?: string | null,
): string {
	if (subject.kind === "everyone") return "@everyone";
	if (subject.kind === "role") {
		const role = data.roles.find((role) => role.role_id === subject.id);
		return role ? `@${role.name}` : `Deleted role ${subject.id}`;
	}
	return known ?? `User ${subject.id}`;
}

/**
 * How many recordings each member may keep within the server's cap: for
 * @everyone, a role or a member.
 */
export function KeepAllowancesPanel({ guildId }: { guildId: string }) {
	const query = useGetKeepAllowancesQuery(guildId, { skip: !guildId });
	const [added, setAdded] = useState<{ subject: RuleSubject; name: string }[]>(
		[],
	);
	const data = query.data;

	const listed: Listed[] = [];
	const list = (entry: Listed) => {
		if (
			!listed.some(
				(other) => subjectKey(other.subject) === subjectKey(entry.subject),
			)
		)
			listed.push(entry);
	};
	if (data) {
		for (const allowance of data.allowances)
			list({
				subject: allowance.subject,
				name: allowanceName(allowance.subject, data, allowance.name),
				saved: allowance.allowance,
				reachesNobody: allowance.reaches_nobody,
			});
		list({
			subject: EVERYONE,
			name: "@everyone",
			saved: null,
			reachesNobody: false,
		});
		for (const entry of added)
			list({ ...entry, saved: null, reachesNobody: false });
		listed.sort(
			(a, b) =>
				Number(b.subject.kind === "everyone") -
				Number(a.subject.kind === "everyone"),
		);
	}
	const unlistedRoles =
		data?.roles.filter(
			(role) =>
				!listed.some(
					(entry) =>
						entry.subject.kind === "role" && entry.subject.id === role.role_id,
				),
		) ?? [];

	return (
		<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-6">
			<h6 className="font-medium tracking-[0.001em] text-xl mb-2">
				Keeping recordings
			</h6>
			{query.isError && (
				<LoadFailure
					error={query.error}
					what="Could not load the keeping allowances."
					onRetry={() => void query.refetch()}
				/>
			)}
			{data && (
				<div className="flex flex-col gap-4">
					<p className="leading-6 text-muted">
						Members keep recordings past retention, and a kept recording stays
						for everyone who may hear it. This server may keep{" "}
						{recordings(data.cap)} and keeps {data.kept}; the bot's operators
						set the cap. Allowances share it out: a member's own allowance wins,
						else the largest of their roles', else @everyone's. With none,
						members keep until the cap is reached.
					</p>
					<ul aria-label="Allowances" className="flex flex-col gap-3">
						{listed.map((entry) => (
							<AllowanceRow
								key={subjectKey(entry.subject)}
								guildId={guildId}
								entry={entry}
							/>
						))}
					</ul>
					<div className="grid gap-3 sm:grid-cols-2">
						{unlistedRoles.length > 0 && (
							<Select
								label="Add a role"
								selectedKey={null}
								onSelectionChange={(key) => {
									const role = data.roles.find((role) => role.role_id === key);
									if (role)
										setAdded((list) => [
											...list,
											{
												subject: { kind: "role", id: role.role_id },
												name: `@${role.name}`,
											},
										]);
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
							onPick={(id, name) =>
								setAdded((list) => [
									...list,
									{ subject: { kind: "user", id }, name },
								])
							}
						/>
					</div>
				</div>
			)}
		</div>
	);
}

function AllowanceRow({ guildId, entry }: { guildId: string; entry: Listed }) {
	const [draft, setDraft] = useState<string | null>(null);
	const [setAllowance, state] = useSetKeepAllowanceMutation();
	const value = draft ?? (entry.saved === null ? "" : String(entry.saved));
	const parsed = value.trim() === "" ? null : Number(value);
	const valid =
		parsed === null ||
		(Number.isInteger(parsed) && parsed >= 0 && parsed <= MAX_ALLOWANCE);
	const dirty = draft !== null && parsed !== entry.saved;

	const save = async (allowance: number | null) => {
		state.reset();
		try {
			await setAllowance({
				guild_id: guildId,
				subject: entry.subject,
				allowance,
			}).unwrap();
			setDraft(null);
		} catch {
			// The failure shows below; the draft stays.
		}
	};

	return (
		<li className="flex flex-col gap-2">
			<div className="flex flex-col gap-2 min-[600px]:flex-row min-[600px]:items-end">
				<TextField
					label={entry.name}
					type="number"
					min={0}
					max={MAX_ALLOWANCE}
					value={value}
					onChange={setDraft}
					isDisabled={entry.reachesNobody}
					error={
						valid
							? undefined
							: `A whole number from 0 to ${MAX_ALLOWANCE}, or blank.`
					}
					description={
						entry.reachesNobody
							? "This role is gone: the allowance reaches nobody."
							: entry.subject.kind === "everyone"
								? "Blank: no limit but the server's cap."
								: "Blank: no allowance of its own."
					}
					className="min-[600px]:w-64"
				/>
				<div className="flex gap-2 min-[600px]:pb-6">
					<Button
						variant="primary"
						isDisabled={!dirty || !valid}
						isPending={state.isLoading}
						onPress={() => void save(parsed)}
					>
						Save
					</Button>
					{entry.saved !== null && (
						<IconButton
							variant="danger"
							aria-label={`Remove the allowance for ${entry.name}`}
							isDisabled={state.isLoading}
							onPress={() => void save(null)}
						>
							<Trash2 aria-hidden="true" className="size-4" />
						</IconButton>
					)}
				</div>
			</div>
			{state.isSuccess && (
				<Notice tone="success" announce="status">
					Allowance saved.
				</Notice>
			)}
			{state.isError && (
				<Notice tone="error" announce="alert">
					{failure(state.error, "The allowance was not saved.")}
				</Notice>
			)}
		</li>
	);
}
