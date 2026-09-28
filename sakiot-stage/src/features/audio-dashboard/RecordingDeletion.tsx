import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { type ApiProblem, problemFromQueryError } from "../../app/apiError";
import {
	type RecordingDeletionStatus as DeletionStatus,
	useDeleteRecordingMutation,
	useGetRecordingDeletionQuery,
} from "../../app/apiSlice";
import { Button, Notice } from "../../shared/ui";

export type RecordingDeletionJob = {
	guild_id: string;
	job_id: string;
	/** The status the removal request answered with. */
	initial?: DeletionStatus;
};

function removalFailure(problem: ApiProblem): string {
	if (problem.kind === "forbidden" || problem.status === 403)
		return "Removing recordings requires the Manage Server permission.";
	if (problem.kind === "media_not_found" || problem.status === 404)
		return "This recording no longer exists. It may already have been removed.";
	// Conflicts (such as a recording that is still in progress) explain
	// themselves; everything else is already worded for display.
	return `Could not remove the recording. ${problem.message}`;
}

export function RemoveRecordingControl(props: {
	guildId: string;
	sessionId: string;
	onRemoved: (job: RecordingDeletionJob) => void;
}) {
	const [deleteRecording, deleteRecordingState] = useDeleteRecordingMutation();
	const [unreadableReply, setUnreadableReply] = useState(false);
	return (
		<div className="mb-4 space-y-2">
			<Button
				variant="danger"
				isDisabled={deleteRecordingState.isLoading}
				onPress={() => {
					if (
						!window.confirm(
							"Remove this recording and its clips from view? Media and metadata will be retained.",
						)
					)
						return;
					setUnreadableReply(false);
					void deleteRecording({
						guild_id: props.guildId,
						recording_session_id: props.sessionId,
					})
						.unwrap()
						.then((job) => {
							// The request succeeded; without an id there is nothing
							// to track, and guessing a state would mislead.
							const readable = typeof job?.id === "string";
							setUnreadableReply(!readable);
							if (readable)
								props.onRemoved({
									guild_id: props.guildId,
									job_id: job.id,
									initial: job,
								});
						})
						// The mutation state renders the failure below.
						.catch(() => {});
				}}
			>
				Remove recording from view
			</Button>
			{unreadableReply && (
				<Notice tone="warning" announce="alert">
					The server accepted the removal, but its reply could not be read.
					Reload the page to see the recording's current state.
				</Notice>
			)}
			{deleteRecordingState.isError && (
				<Notice tone="error" announce="alert">
					{removalFailure(problemFromQueryError(deleteRecordingState.error))}
				</Notice>
			)}
		</div>
	);
}

const TERMINAL_STATES = new Set(["soft_deleted", "ready", "failed", "paused"]);

function describe(status: DeletionStatus): {
	tone: "info" | "success" | "error" | "warning";
	text: string;
} {
	switch (status.state) {
		case "soft_deleted":
			return {
				tone: "success",
				text: "Recording removed from view. Its media and metadata are retained.",
			};
		case "ready":
			return {
				tone: "success",
				text: "Recording and its media were permanently deleted.",
			};
		case "failed":
		case "paused":
			return {
				tone: "error",
				text: `Permanent deletion stopped and needs administrator review. The recording remains hidden.${status.error ? ` ${status.error}` : ""}`,
			};
		case "queued":
		case "running":
			return {
				tone: "info",
				text: `Recording hidden. Permanent deletion is in progress.${status.error ? ` ${status.error}` : ""}`,
			};
		default:
			return {
				tone: "warning",
				text: "Recording hidden. Its deletion status is not recognized by this page; reload to check again.",
			};
	}
}

/** Replaces the player once a recording has been removed from view. */
export function RecordingDeletionStatus(props: { job: RecordingDeletionJob }) {
	const navigate = useNavigate();
	const { initial, ...job } = props.job;
	const query = useGetRecordingDeletionQuery(job, {
		pollingInterval: initial && TERMINAL_STATES.has(initial.state) ? 0 : 2_000,
	});
	// Fall back on the confirmed answer to the removal request, never on a
	// guess: without either, the page says it does not know.
	const status = query.data ?? initial;
	const problem = query.isError ? problemFromQueryError(query.error) : null;
	const described = status ? describe(status) : null;
	return (
		<div className="p-4 space-y-3">
			{described ? (
				<Notice tone={described.tone} announce="status">
					{described.text}
				</Notice>
			) : query.isLoading ? (
				<Notice announce="status">Checking the deletion status…</Notice>
			) : null}
			{problem && (
				<Notice tone="warning" announce="alert">
					{status
						? `Could not refresh the deletion status; showing the last known state. ${problem.message}`
						: `The recording was removed from view, but its deletion status could not be loaded. ${problem.message}`}
				</Notice>
			)}
			<div className="flex flex-wrap gap-2">
				{problem && (
					<Button variant="outline" onPress={() => void query.refetch()}>
						Check again
					</Button>
				)}
				<Button
					variant="outline"
					onPress={() => navigate(`/dashboard/${props.job.guild_id}/audio`)}
				>
					Back to recordings
				</Button>
			</div>
		</div>
	);
}
