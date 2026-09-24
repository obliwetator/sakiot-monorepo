import { useState } from "react";
import type { Params } from "react-router-dom";
import { API_ROUTES, apiUrl } from "../../../api/routes";
import { isDefiniteRejection, problemFromError } from "../../../app/apiError";
import {
	type CreateClipResponse,
	useCreateClipMutation,
} from "../../../app/apiSlice";
import { downloadFile } from "../../../app/download";
import type { AudioParams } from "../../../Constants";
import { BaseDialog } from "../../../shared/BaseDialog";
import { Button, TextField } from "../../../shared/ui";
import type { StartEnd } from "./useRangeSliderState";

export function ClipDialog(props: {
	params: Readonly<Params<AudioParams>>;
	startEnd: StartEnd;
	disabled: boolean;
}) {
	const [open, setOpen] = useState(false);
	const [text, setText] = useState("");
	const [errorMsg, setErrorMsg] = useState("");
	const [downloading, setDownloading] = useState(false);
	// Once the server creates the clip, recovery only retries the download:
	// pressing the button again must never create a duplicate.
	const [created, setCreated] = useState<CreateClipResponse | null>(null);
	const [createClip, { isLoading }] = useCreateClipMutation();
	const busy = isLoading || downloading;

	const handleClickOpen = () => {
		setOpen(true);
		setText("");
		setErrorMsg("");
		setCreated(null);
	};

	const handleClose = () => setOpen(false);

	const download = async (clip: CreateClipResponse) => {
		setDownloading(true);
		setErrorMsg("");
		try {
			await downloadFile(
				apiUrl(API_ROUTES.clip, {
					guild_id: props.params.guild_id ?? "",
					clip_id: clip.id,
				}),
				`${clip.name || clip.id}.ogg`,
			);
			setOpen(false);
		} catch (failure) {
			const problem = problemFromError(failure);
			setErrorMsg(
				`The clip "${clip.name || clip.id}" was created and is in your clips, but downloading it failed. ${problem.message}`,
			);
		} finally {
			setDownloading(false);
		}
	};

	const handleClip = async () => {
		if (created) {
			await download(created);
			return;
		}
		if (props.startEnd[1] - props.startEnd[0] > 20) {
			setErrorMsg("Clip duration cannot exceed 20 seconds.");
			return;
		}
		setErrorMsg("");
		let response: CreateClipResponse;
		try {
			response = await createClip({
				guild_id: props.params.guild_id ?? "",
				channel_id: props.params.channel_id ?? "",
				year: props.params.year ?? "",
				month: Number(props.params.month ?? ""),
				file_name: props.params.file_name ?? "",
				start: props.startEnd[0],
				end: props.startEnd[1],
				name: text.length > 0 ? text : undefined,
			}).unwrap();
		} catch (failure) {
			const problem = problemFromError(failure);
			setErrorMsg(
				isDefiniteRejection(problem)
					? `The clip was not created. ${problem.message}`
					: `Could not confirm whether the clip was created. ${problem.message} Check your clips before trying again.`,
			);
			return;
		}
		setCreated(response);
		await download(response);
	};

	return (
		<>
			<Button
				variant="primary"
				isDisabled={props.disabled}
				onPress={handleClickOpen}
			>
				Clip
			</Button>
			<BaseDialog
				open={open}
				onClose={handleClose}
				title="Create clip"
				error={errorMsg}
				busy={busy}
				cancelLabel={created ? "Close" : "Cancel"}
				confirmLabel={
					isLoading
						? "Creating..."
						: downloading
							? "Downloading..."
							: created
								? "Retry download"
								: "Clip"
				}
				onConfirm={handleClip}
			>
				<p className="text-sm leading-6 text-slate-200">
					Enter a name for this clip. Will return an error if name is a
					duplicate. Leave blank for default name
				</p>
				<TextField
					value={text}
					autoFocus
					id="name"
					label="Name"
					type="text"
					autoComplete="off"
					isDisabled={busy || created !== null}
					onChange={(value) => setText(value)}
				/>
			</BaseDialog>
		</>
	);
}
