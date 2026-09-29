import { Music as MusicNoteIcon } from "lucide-react";
import { useState } from "react";
import { type ApiProblem, problemFromError } from "../../app/apiError";
import { usePlayClipMutation } from "../../app/apiSlice";
import { BaseDialog } from "../../shared/BaseDialog";
import { Button, Notice } from "../../shared/ui";

/** The bot queued the clip, or the reason it did not. */
type PlayOutcome = { queued: true } | { queued: false; problem: ApiProblem };

function feedbackFor(outcome: PlayOutcome): { title: string; message: string } {
	if (outcome.queued) {
		return {
			title: "Clip queued",
			message: "Clip was sent to the bot's current voice channel.",
		};
	}
	const { problem } = outcome;
	switch (problem.kind) {
		case "bot_not_in_voice":
			return {
				title: "Bot not connected",
				message: "Bot is not connected to a voice channel in this guild.",
			};
		case "jam_cooldown": {
			const seconds = problem.retryAfterSeconds;
			return {
				title: "On cooldown",
				message:
					seconds === undefined
						? "Try again in a few seconds."
						: `Try again in ${seconds} ${seconds === 1 ? "second" : "seconds"}.`,
			};
		}
		case "bot_unavailable":
			return {
				title: "Could not play clip",
				message:
					"The bot could not be reached. It may be restarting; try again shortly.",
			};
		default:
			return { title: "Could not play clip", message: problem.message };
	}
}

/** Plays a clip into the bot's current voice channel in the clip's guild. */
export function JamIt(props: { guildId: string; clipId: string }) {
	const [outcome, setOutcome] = useState<PlayOutcome | null>(null);
	const [playClip, playState] = usePlayClipMutation();

	const handleJamIt = async () => {
		try {
			await playClip({
				guild_id: props.guildId,
				clip_id: props.clipId,
			}).unwrap();
			setOutcome({ queued: true });
		} catch (err: unknown) {
			setOutcome({
				queued: false,
				problem: problemFromError(
					err,
					"Clip playback failed. Try again shortly.",
				),
			});
		}
	};

	const feedback = outcome ? feedbackFor(outcome) : null;
	return (
		<>
			<Button
				variant="primary"
				isDisabled={playState.isLoading}
				onPress={() => void handleJamIt()}
			>
				<MusicNoteIcon />
				Jam It
			</Button>
			{outcome && feedback && (
				<BaseDialog
					open={true}
					onClose={() => setOutcome(null)}
					title={feedback.title}
				>
					<Notice
						tone={outcome.queued ? "success" : "warning"}
						announce="status"
					>
						{feedback.message}
					</Notice>
				</BaseDialog>
			)}
		</>
	);
}
