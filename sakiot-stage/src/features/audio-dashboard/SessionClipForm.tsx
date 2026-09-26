import { useState } from "react";
import { useCreateSessionClipMutation } from "../../app/apiSlice";
import { Button, Notice, TextField } from "../../shared/ui";
import type { SessionSelection } from "./logicalSessionSelection";

/** Names and creates a clip from the current selection. */
export function SessionClipForm(props: {
	sessionId: string;
	selection: SessionSelection;
	silenceFree: boolean;
}) {
	const [clipName, setClipName] = useState("");
	const [message, setMessage] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [createClip, clipState] = useCreateSessionClipMutation();

	const createSelectedClip = async () => {
		setError(null);
		setMessage(null);
		try {
			const response = await createClip({
				recording_session_id: props.sessionId,
				start: props.selection[0] / 1_000,
				end: props.selection[1] / 1_000,
				name: clipName.trim() || undefined,
				silence_free: props.silenceFree,
			}).unwrap();
			setMessage(`Clip created: ${response.name}`);
			setClipName("");
		} catch {
			setError("Clip creation failed. Select 1-20 seconds.");
		}
	};

	return (
		<>
			<div className="flex items-end flex-nowrap flex-row gap-2 min-w-0">
				<TextField
					label="Clip name"
					value={clipName}
					className="flex-1 min-w-0"
					onChange={(value) => setClipName(value)}
					inputClassName="h-10 shrink-0"
				/>
				<Button
					className="h-10 flex-none"
					variant="primary"
					isDisabled={clipState.isLoading}
					onPress={() => void createSelectedClip()}
				>
					Create clip
				</Button>
			</div>
			{message && (
				<Notice className="mt-4" tone={"success"} announce="status">
					{message}
				</Notice>
			)}
			{error && (
				<Notice className="mt-4" tone={"error"} announce="alert">
					{error}
				</Notice>
			)}
		</>
	);
}
