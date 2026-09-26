import { Button, Notice, ProgressBar, WithTooltip } from "../../shared/ui";
import type { useSilenceRemoval } from "./useSilenceRemoval";

/** Session downloads and silence removal, with their progress and outcome. */
export function SessionMediaActions(props: {
	removal: ReturnType<typeof useSilenceRemoval>;
	finalized: boolean;
	/** A playback error to show alongside the removal's own error. */
	playbackError: string | null;
}) {
	const { removal } = props;
	const { action } = removal;
	const processing = removal.status.status === "processing";
	const hasSilenceFree = Boolean(removal.mediaUrl);
	const error = removal.error ?? props.playbackError;
	return (
		<>
			<div className="flex items-center flex-wrap flex-row gap-2 mt-2">
				<Button
					variant="outline"
					isDisabled={action !== null}
					onPress={() => void removal.downloadSession()}
				>
					{action === "download" ? "Preparing…" : "Download session"}
				</Button>
				{!hasSilenceFree && (
					<WithTooltip
						tip={
							props.finalized
								? undefined
								: "Silence removal is available after the recording is finalized"
						}
					>
						<Button
							variant="primary"
							isDisabled={action !== null || processing || !props.finalized}
							onPress={() => void removal.create()}
						>
							{action === "silence" || processing
								? `Removing silence… ${removal.status.progress}%`
								: "Remove silence"}
						</Button>
					</WithTooltip>
				)}
				{hasSilenceFree && (
					<Button
						variant="outline"
						isDisabled={action !== null || processing}
						onPress={() => void removal.create(true)}
					>
						{action === "silence" ? "Regenerating…" : "Regenerate silence-free"}
					</Button>
				)}
				{hasSilenceFree && (
					<Button
						variant="outline"
						isDisabled={action !== null}
						onPress={() => void removal.downloadSilenceFree()}
					>
						{action === "silence-download"
							? "Preparing…"
							: "Download silence-free"}
					</Button>
				)}
			</div>
			{processing && (
				<div className="mt-2 max-w-140">
					<div className="flex justify-between mb-1 flex-row">
						<p className="text-sm">Removing silence</p>
						<p className="text-sm">{removal.status.progress}%</p>
					</div>
					<ProgressBar
						value={removal.status.progress}
						aria-label="Silence removal progress"
					/>
					<span className="text-muted text-xs leading-5">
						This can continue in the background; progress resumes if you refresh
						the page.
					</span>
				</div>
			)}
			{removal.message && (
				<Notice className="mt-2" tone={"success"} announce="status">
					{removal.message}
				</Notice>
			)}
			{error && (
				<Notice className="mt-2" tone={"error"} announce="alert">
					{error}
				</Notice>
			)}
		</>
	);
}
