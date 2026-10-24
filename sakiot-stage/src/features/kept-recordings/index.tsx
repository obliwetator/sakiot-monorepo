import { Link as RouterLink, useParams } from "react-router-dom";
import { problemFromError } from "../../app/apiError";
import {
	type Keeping,
	type KeptRecording,
	type RemovedRecording,
	useGetAuthDetailsQuery,
	useGetKeepingQuery,
	useGetRemovedRecordingsInfiniteQuery,
	useKeepRecordingMutation,
	useReleaseRecordingForEveryoneMutation,
	useReleaseRecordingMutation,
	useRestoreRecordingMutation,
} from "../../app/apiSlice";
import { PATH_PREFIX_FOR_LOGGED_USERS } from "../../Constants";
import { LoadFailure } from "../../shared/LoadFailure";
import { holds, isFormerMember } from "../../shared/permissions";
import {
	Badge,
	Button,
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Text,
} from "../../shared/ui";
import {
	keepBlocked,
	keepers,
	recordingLabel,
	removal,
	serverUsage,
	viewerUsage,
	when,
} from "./keeping";

/**
 * Keeping recordings past retention: what the server and the viewer may
 * keep, what is kept, and the removed recordings the viewer may bring back.
 */
export function KeptRecordingsPage() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";
	const { data: auth } = useGetAuthDetailsQuery();
	const guild = auth?.guilds?.find((guild) => guild.id === gid) ?? null;
	const former = isFormerMember(guild);
	const manager = holds(guild, "delete_recordings");
	const keeping = useGetKeepingQuery(gid, { skip: !gid });

	if (!gid) {
		return (
			<Page>
				<Notice tone="error">Missing guild id.</Notice>
			</Page>
		);
	}

	const data = keeping.data;
	return (
		<Page className="space-y-5">
			<header className="space-y-1">
				<PageTitle>Kept recordings</PageTitle>
				<Text tone="muted">
					{data?.retention_days
						? `Retention removes recordings ${data.retention_days} days after they end. A kept recording stays, for everyone who may hear it, until nobody keeps it.`
						: "Retention is off in this server: no recording is removed for its age. A kept recording stays if it is ever turned on."}
				</Text>
			</header>

			{keeping.isLoading && <Text aria-live="polite">Loading…</Text>}
			{keeping.isError && (
				<LoadFailure
					error={keeping.error}
					what="Could not load the kept recordings."
					onRetry={() => void keeping.refetch()}
				/>
			)}

			{data && (
				<>
					<Panel aria-labelledby="kept-heading" className="space-y-4">
						<div className="space-y-1">
							<SectionTitle id="kept-heading">Kept</SectionTitle>
							<Text>{serverUsage(data)}</Text>
							{!former && <Text>{viewerUsage(data)}</Text>}
						</div>
						<KeptList guildId={gid} keeping={data} manager={manager} />
					</Panel>
					{former ? (
						<Notice tone="info">
							Only members keep recordings. You can still stop keeping yours.
						</Notice>
					) : (
						<RemovedPanel guildId={gid} keeping={data} manager={manager} />
					)}
				</>
			)}
		</Page>
	);
}

function KeptList({
	guildId,
	keeping,
	manager,
}: {
	guildId: string;
	keeping: Keeping;
	manager: boolean;
}) {
	if (keeping.recordings.length === 0)
		return <Text tone="muted">Nothing is kept yet.</Text>;
	return (
		<ul aria-label="Kept recordings" className="flex flex-col gap-2">
			{keeping.recordings.map((recording) => (
				<KeptRow
					key={recording.recording_session_id}
					guildId={guildId}
					recording={recording}
					manager={manager}
				/>
			))}
		</ul>
	);
}

// Row actions show no pending spinner, nor does "Show more": success removes
// the button, and React Aria's pending announcement would outlive it.
function KeptRow({
	guildId,
	recording,
	manager,
}: {
	guildId: string;
	recording: KeptRecording;
	manager: boolean;
}) {
	const [release, released] = useReleaseRecordingMutation();
	const [releaseAll, releasedAll] = useReleaseRecordingForEveryoneMutation();
	const busy = released.isLoading || releasedAll.isLoading;
	const failure = released.error ?? releasedAll.error;
	const id = recording.recording_session_id;
	const label =
		recording.audible && recording.speaker_id && recording.channel_id
			? recordingLabel({
					speaker_id: recording.speaker_id,
					speaker_name: recording.speaker_name ?? null,
					channel_id: recording.channel_id,
					channel_name: recording.channel_name ?? null,
					started_at_ms: recording.started_at_ms,
				})
			: "A recording you can no longer hear";
	const args = { guild_id: guildId, recording_session_id: id };
	return (
		<li className="space-y-2 rounded-lg border border-ui-border p-3">
			<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
				<div className="min-w-0 flex-1">
					<p className="truncate text-sm font-medium text-fg">
						{recording.audible ? (
							<RouterLink
								to={`${PATH_PREFIX_FOR_LOGGED_USERS}/${guildId}/audio/session/${encodeURIComponent(id)}`}
								className="text-accent underline-offset-2 hover:underline"
							>
								{label}
							</RouterLink>
						) : (
							label
						)}
					</p>
					<p className="text-xs text-muted">
						{keepers(recording)} · since {when(recording.kept_at_ms)}
					</p>
				</div>
				<div className="flex flex-wrap gap-2">
					{recording.mine && (
						<Button
							variant="outline"
							size="sm"
							aria-label={`Stop keeping ${label}`}
							isDisabled={busy}
							onPress={() => void release(args)}
						>
							Stop keeping
						</Button>
					)}
					{manager && (recording.keepers > 1 || !recording.mine) && (
						<Button
							variant="danger"
							size="sm"
							aria-label={`Stop everyone keeping ${label}`}
							isDisabled={busy}
							onPress={() => void releaseAll(args)}
						>
							Stop everyone keeping it
						</Button>
					)}
				</div>
			</div>
			{failure && (
				<Notice tone="error" announce="alert">
					Could not stop keeping it. {problemFromError(failure).message}
				</Notice>
			)}
		</li>
	);
}

function RemovedPanel({
	guildId,
	keeping,
	manager,
}: {
	guildId: string;
	keeping: Keeping;
	manager: boolean;
}) {
	const removed = useGetRemovedRecordingsInfiniteQuery(guildId);
	const recordings = removed.data?.pages.flatMap((page) => page.recordings);
	const blocked = keepBlocked(keeping);
	return (
		<Panel aria-labelledby="removed-heading" className="space-y-4">
			<div className="space-y-1">
				<SectionTitle id="removed-heading">Removed</SectionTitle>
				<Text tone="muted">
					{manager
						? "Recordings retention or a manager removed. Keeping one brings it back for everyone who may hear it; within the retention period, you can also restore what a manager removed."
						: "Recordings retention removed that you may hear. Keeping one brings it back for everyone who may hear it."}
				</Text>
				{blocked && <Text>{blocked}</Text>}
			</div>
			{removed.isLoading && <Text aria-live="polite">Loading…</Text>}
			{removed.isError && (
				<LoadFailure
					error={removed.error}
					what="Could not load the removed recordings."
					onRetry={() => void removed.refetch()}
				/>
			)}
			{recordings &&
				(recordings.length === 0 ? (
					<Text tone="muted">Nothing removed that you could bring back.</Text>
				) : (
					<ul aria-label="Removed recordings" className="flex flex-col gap-2">
						{recordings.map((recording) => (
							<RemovedRow
								key={recording.recording_session_id}
								guildId={guildId}
								recording={recording}
								blocked={blocked}
							/>
						))}
					</ul>
				))}
			{removed.hasNextPage && (
				<Button
					variant="outline"
					isDisabled={removed.isFetchingNextPage}
					onPress={() => void removed.fetchNextPage()}
				>
					Show more
				</Button>
			)}
		</Panel>
	);
}

function RemovedRow({
	guildId,
	recording,
	blocked,
}: {
	guildId: string;
	recording: RemovedRecording;
	blocked: string | null;
}) {
	const [keep, kept] = useKeepRecordingMutation();
	const [restore, restored] = useRestoreRecordingMutation();
	const busy = kept.isLoading || restored.isLoading;
	const failure = kept.error ?? restored.error;
	const label = recordingLabel(recording);
	const args = {
		guild_id: guildId,
		recording_session_id: recording.recording_session_id,
	};
	return (
		<li className="space-y-2 rounded-lg border border-ui-border p-3">
			<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
				<div className="min-w-0 flex-1">
					<p className="flex items-center gap-2 text-sm font-medium text-fg">
						<span className="truncate">{label}</span>
						{recording.redacted && (
							<Badge size="sm" tone="warning">
								Private
							</Badge>
						)}
					</p>
					<p className="text-xs text-muted">
						{removal(recording)}
						{recording.redacted &&
							" · its speaker keeps their recordings private: you can bring it back, not hear it"}
					</p>
				</div>
				<div className="flex flex-wrap gap-2">
					{recording.restorable && (
						<Button
							variant="outline"
							size="sm"
							aria-label={`Restore ${label}`}
							isDisabled={busy}
							onPress={() => void restore(args)}
						>
							Restore
						</Button>
					)}
					<Button
						variant="primary"
						size="sm"
						aria-label={`Keep ${label}`}
						isDisabled={busy || blocked !== null}
						onPress={() => void keep(args)}
					>
						Keep
					</Button>
				</div>
			</div>
			{failure && (
				<Notice tone="error" announce="alert">
					{problemFromError(failure).message}
				</Notice>
			)}
		</li>
	);
}
