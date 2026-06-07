import Button from "@mui/material/Button";
import { useState } from "react";
import { useDispatch } from "react-redux";
import type { Params } from "react-router-dom";
import { useRemoveSilenceMutation } from "../../../app/apiSlice";
import { useAppSelector } from "../../../app/hooks";
import type { AudioParams } from "../../../Constants";
import { bumpSilenceVersion, setHasSilence } from "../../../reducers/silence";

export function SilenceButton(props: {
	params: Readonly<Params<AudioParams>>;
	isSilence: boolean;
	isLive?: boolean;
}) {
	const [isLoading, setIsLoading] = useState(false);
	const [removeSilence] = useRemoveSilenceMutation();
	const dispatch = useDispatch();
	const hasSilence = useAppSelector((state) => state.hasSilence.value);

	const handleOnClick = async () => {
		setIsLoading(true);
		try {
			const payload = {
				guild_id: props.params.guild_id ?? "",
				channel_id: props.params.channel_id ?? "",
				year: props.params.year ?? "",
				month: Number(props.params.month ?? ""),
				file_name: props.params.file_name ?? "",
				idempotency_key: crypto.randomUUID(),
			};
			// First call kicks off generation ("Request Accepted"); subsequent
			// calls hit the "already processing" path which blocks until the
			// ffmpeg pass finishes. Loop until it's no longer just-started.
			let res = await removeSilence(payload).unwrap();
			while (res.message === "Request Accepted") {
				res = await removeSilence(payload).unwrap();
			}
			dispatch(setHasSilence(true));
			// New silence-free file on disk — bust the player's cache so it
			// reloads the regenerated audio in place.
			dispatch(bumpSilenceVersion());
		} catch (error) {
			console.error("Error removing silence:", error);
		} finally {
			setIsLoading(false);
		}
	};

	// On the silence tab itself there's nothing to remove. Otherwise: hide once
	// a silence-free version exists, except while live — then keep it so the
	// user can refresh as the recording grows.
	if (props.isSilence) return null;
	if (hasSilence && !props.isLive) return null;

	const label = hasSilence ? "Refresh silence-free" : "Remove Silence";

	return (
		<Button variant="contained" onClick={handleOnClick} disabled={isLoading}>
			{isLoading ? "Working..." : label}
		</Button>
	);
}
