import { useNavigate } from "react-router-dom";
import {
	useDeleteRecordingMutation,
	useGetRecordingDeletionQuery,
} from "../../app/apiSlice";
import { Button, Notice } from "../../shared/ui";

export type RecordingDeletionJob = { guild_id: string; job_id: string };

export function RemoveRecordingControl(props: {
	guildId: string;
	sessionId: string;
	onRemoved: (job: RecordingDeletionJob) => void;
}) {
	const [deleteRecording, deleteRecordingState] = useDeleteRecordingMutation();
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
					void deleteRecording({
						guild_id: props.guildId,
						recording_session_id: props.sessionId,
					})
						.unwrap()
						.then((job) =>
							props.onRemoved({ guild_id: props.guildId, job_id: job.id }),
						)
						.catch(() => {});
				}}
			>
				Remove recording from view
			</Button>
			{deleteRecordingState.isError && (
				<Notice tone="error" announce="alert">
					Could not remove the recording. Try again.
				</Notice>
			)}
		</div>
	);
}

/** Replaces the player once a recording has been removed from view. */
export function RecordingDeletionStatus(props: { job: RecordingDeletionJob }) {
	const navigate = useNavigate();
	const { data: status } = useGetRecordingDeletionQuery(props.job, {
		pollingInterval: 2_000,
	});
	const failed = status?.state === "failed" || status?.state === "paused";
	return (
		<div className="p-4 space-y-3">
			<Notice tone={failed ? "error" : "info"} announce="status">
				{status?.state === "soft_deleted"
					? "Recording removed from view. Its media and metadata are retained."
					: status?.state === "ready"
						? "Recording and its media were permanently deleted."
						: failed
							? "A previous permanent deletion needs administrator review. The recording remains hidden."
							: "Recording hidden. Permanent deletion is in progress."}
			</Notice>
			<Button
				variant="outline"
				onPress={() => navigate(`/dashboard/${props.job.guild_id}/audio`)}
			>
				Back to recordings
			</Button>
		</div>
	);
}
