import { useState } from "react";
import { problemFromError } from "../../app/apiError";
import {
	type OperatorGuild,
	useGetAuthDetailsQuery,
	useGetOperatorGuildsQuery,
	useSetGuildLimitsMutation,
} from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import {
	Badge,
	Button,
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Text,
	TextField,
} from "../../shared/ui";
import { recordings } from "../kept-recordings/keeping";

/** The server's `MAX_KEPT`. */
const MAX_CAP = 100_000;

/**
 * What the bot's operators allow each guild, which nobody in a guild can
 * change. Operators are added on the server (`web-server operators add`).
 */
export function OperatorPage() {
	const { data: auth } = useGetAuthDetailsQuery();
	const operator = auth?.user?.is_operator === true;
	const guilds = useGetOperatorGuildsQuery(undefined, { skip: !operator });

	return (
		<Page className="space-y-5">
			<header className="space-y-1">
				<PageTitle>Operator</PageTitle>
				<Text tone="muted">
					Limits for every server the bot is in or holds recordings of. Only the
					bot's operators see this page; nobody in a server can change what is
					set here.
				</Text>
			</header>
			{auth && !operator && (
				<Notice tone="warning">
					Only the bot's operators can open this page.
				</Notice>
			)}
			{guilds.isError && (
				<LoadFailure
					error={guilds.error}
					what="Could not load the servers."
					onRetry={() => void guilds.refetch()}
				/>
			)}
			{guilds.data && (
				<Panel aria-labelledby="servers-heading" className="space-y-4">
					<SectionTitle id="servers-heading">Servers</SectionTitle>
					{guilds.data.length === 0 ? (
						<Text tone="muted">The bot knows no servers yet.</Text>
					) : (
						<ul aria-label="Servers" className="flex flex-col gap-3">
							{guilds.data.map((guild) => (
								<GuildLimitsRow key={guild.guild_id} guild={guild} />
							))}
						</ul>
					)}
				</Panel>
			)}
		</Page>
	);
}

function GuildLimitsRow({ guild }: { guild: OperatorGuild }) {
	const [draft, setDraft] = useState<string | null>(null);
	const [setLimits, state] = useSetGuildLimitsMutation();
	const name = guild.name ?? `Server ${guild.guild_id}`;
	const value = draft ?? String(guild.kept_recordings_cap);
	const parsed = Number(value);
	const valid =
		value.trim() !== "" &&
		Number.isInteger(parsed) &&
		parsed >= 0 &&
		parsed <= MAX_CAP;
	const dirty = draft !== null && parsed !== guild.kept_recordings_cap;

	const save = async () => {
		state.reset();
		try {
			await setLimits({
				guild_id: guild.guild_id,
				kept_recordings: parsed,
			}).unwrap();
			setDraft(null);
		} catch {
			// The failure shows below; the draft stays.
		}
	};

	return (
		<li
			aria-label={name}
			className="space-y-3 rounded-lg border border-ui-border p-4"
		>
			<div className="flex flex-wrap items-center gap-2">
				<p className="text-sm font-semibold text-fg">{name}</p>
				{!guild.present && (
					<Badge size="sm" tone="neutral">
						Bot not in it
					</Badge>
				)}
			</div>
			<p className="text-xs text-muted">
				Keeps {guild.kept_recordings} of {recordings(guild.kept_recordings_cap)}
				.{" "}
				{guild.retention_days
					? `Retention removes recordings after ${guild.retention_days} days.`
					: "Retention is off."}
			</p>
			<div className="flex flex-col gap-2 min-[600px]:flex-row min-[600px]:items-end">
				<TextField
					label="Recordings it may keep"
					type="number"
					min={0}
					max={MAX_CAP}
					value={value}
					onChange={setDraft}
					error={valid ? undefined : `A whole number from 0 to ${MAX_CAP}.`}
					description={
						parsed < guild.kept_recordings && valid
							? "Below what it keeps: those stay kept, and nothing more can be until it is under the cap."
							: undefined
					}
					className="min-[600px]:w-64"
				/>
				<Button
					variant="primary"
					isDisabled={!dirty || !valid}
					isPending={state.isLoading}
					onPress={() => void save()}
					className="min-[600px]:mb-px"
				>
					Save
				</Button>
			</div>
			{state.isSuccess && (
				<Notice tone="success" announce="status">
					Saved.
				</Notice>
			)}
			{state.isError && (
				<Notice tone="error" announce="alert">
					The limit was not saved. {problemFromError(state.error).message}
				</Notice>
			)}
		</li>
	);
}
