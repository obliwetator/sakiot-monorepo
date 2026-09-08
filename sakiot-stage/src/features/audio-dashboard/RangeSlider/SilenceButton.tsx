import { useEffect, useRef, useState } from "react";
import { useDispatch } from "react-redux";
import type { Params } from "react-router-dom";
import { useRemoveSilenceMutation } from "../../../app/apiSlice";
import { useAppSelector } from "../../../app/hooks";
import type { AudioParams } from "../../../Constants";
import { bumpSilenceVersion, setHasSilence } from "../../../reducers/silence";
import { Button, Notice } from "../../../shared/ui";
import { SilenceJobTimeoutError, waitForSilenceJob } from "./silenceJobPoll";

export function SilenceButton(props: {
	params: Readonly<Params<AudioParams>>;
	isSilence: boolean;
	isLive?: boolean;
}) {
	const [isLoading, setIsLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [removeSilence] = useRemoveSilenceMutation();
	const dispatch = useDispatch();
	const hasSilence = useAppSelector((state) => state.hasSilence.value);
	const abortRef = useRef<AbortController | null>(null);

	// A job that is still waiting on the server must not keep polling after the
	// component is gone (navigating to another recording unmounts this button).
	useEffect(() => () => abortRef.current?.abort(), []);

	const handleOnClick = async () => {
		abortRef.current?.abort();
		const controller = new AbortController();
		abortRef.current = controller;
		setIsLoading(true);
		setError(null);
		try {
			// One idempotency key for the whole wait: the first call starts the
			// job, the retry blocks on the same job instead of starting another.
			const payload = {
				guild_id: props.params.guild_id ?? "",
				channel_id: props.params.channel_id ?? "",
				year: props.params.year ?? "",
				month: Number(props.params.month ?? ""),
				file_name: props.params.file_name ?? "",
				idempotency_key: crypto.randomUUID(),
			};
			await waitForSilenceJob({
				request: () => removeSilence(payload).unwrap(),
				signal: controller.signal,
			});
			dispatch(setHasSilence(true));
			// New silence-free file on disk — bust the player's cache so it
			// reloads the regenerated audio in place.
			dispatch(bumpSilenceVersion());
		} catch (err) {
			if (controller.signal.aborted) return;
			console.error("Error removing silence:", err);
			setError(
				err instanceof SilenceJobTimeoutError
					? "Silence removal is taking longer than expected. Try again."
					: "Could not remove silence. Try again.",
			);
		} finally {
			if (abortRef.current === controller) {
				abortRef.current = null;
				setIsLoading(false);
			}
		}
	};

	// On the silence tab itself there's nothing to remove. Otherwise: hide once
	// a silence-free version exists, except while live — then keep it so the
	// user can refresh as the recording grows.
	if (props.isSilence) return null;
	if (hasSilence && !props.isLive) return null;

	const label = hasSilence ? "Refresh silence-free" : "Remove Silence";

	return (
		<>
			<Button variant="primary" isDisabled={isLoading} onPress={handleOnClick}>
				{isLoading ? "Working..." : label}
			</Button>
			{error ? (
				<Notice tone="error" announce="alert">
					{error}
				</Notice>
			) : null}
		</>
	);
}
