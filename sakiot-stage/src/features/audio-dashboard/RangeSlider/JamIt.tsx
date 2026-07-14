import MusicNoteIcon from "@mui/icons-material/MusicNote";
import Alert from "@mui/material/Alert";
import Button from "@mui/material/Button";
import { useState } from "react";
import { useParams } from "react-router-dom";
import { useJamItMutation } from "../../../app/apiSlice";
import { BaseDialog } from "../../../shared/BaseDialog";

export enum JamItRespStatus {
	OK,
	NOT_CONNECTED,
	UNKNOWN,
	COOLDOWN,
}

export function JamIt(props: { visible: boolean }) {
	const [feedback, setFeedback] = useState<{
		type: JamItRespStatus;
		cooldownRemaining?: number;
	} | null>(null);
	const params = useParams();
	const [jamIt, jamState] = useJamItMutation();
	const [open, setOpen] = useState(false);

	if (!props.visible) return null;

	const handleJamIt = async () => {
		try {
			const res = await jamIt({
				guild_id: params.guild_id as string,
				clip_name: params.file_name as string,
			}).unwrap();

			const code = Number(res.code);
			setFeedback({
				type:
					code === JamItRespStatus.OK || code === JamItRespStatus.NOT_CONNECTED
						? code
						: JamItRespStatus.UNKNOWN,
			});
			setOpen(true);
		} catch (err: unknown) {
			const apiError = err as {
				status?: number;
				data?: { code?: number; cooldown_remaining_seconds?: number };
			};
			if (apiError?.status === 429) {
				setFeedback({
					type: JamItRespStatus.COOLDOWN,
					cooldownRemaining: apiError?.data?.cooldown_remaining_seconds,
				});
			} else {
				setFeedback({ type: JamItRespStatus.UNKNOWN });
			}
			setOpen(true);
		}
	};

	const handleClose = () => {
		setOpen(false);
		setFeedback(null);
	};

	const title =
		feedback?.type === JamItRespStatus.OK
			? "Clip queued"
			: feedback?.type === JamItRespStatus.NOT_CONNECTED
				? "Bot not connected"
				: feedback?.type === JamItRespStatus.COOLDOWN
					? "On cooldown"
					: "Could not play clip";
	const message =
		feedback?.type === JamItRespStatus.OK
			? "Clip was sent to the bot's current voice channel."
			: feedback?.type === JamItRespStatus.NOT_CONNECTED
				? "Bot is not connected to a voice channel in this guild."
				: feedback?.type === JamItRespStatus.COOLDOWN
					? `Try again in ${feedback.cooldownRemaining ?? "a few"} seconds.`
					: "Clip playback failed. Try again shortly.";

	return (
		<>
			<Button
				onClick={() => void handleJamIt()}
				variant="contained"
				startIcon={<MusicNoteIcon />}
				disabled={jamState.isLoading}
			>
				Jam It
			</Button>
			{feedback && (
				<BaseDialog open={open} onClose={handleClose} title={title}>
					<Alert
						severity={
							feedback.type === JamItRespStatus.OK ? "success" : "warning"
						}
					>
						{message}
					</Alert>
				</BaseDialog>
			)}
		</>
	);
}
