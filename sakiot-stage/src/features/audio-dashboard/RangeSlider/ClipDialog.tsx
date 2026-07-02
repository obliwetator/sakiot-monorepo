import Button from "@mui/material/Button";
import DialogContentText from "@mui/material/DialogContentText";
import TextField from "@mui/material/TextField";
import { useState } from "react";
import type { Params } from "react-router-dom";
import { useCreateClipMutation } from "../../../app/apiSlice";
import { authedFetch } from "../../../app/authedFetch";
import type { AudioParams } from "../../../Constants";
import { BaseDialog } from "../../../shared/BaseDialog";

export function ClipDialog(props: {
	params: Readonly<Params<AudioParams>>;
	startEnd: number[];
	disabled: boolean;
}) {
	const [open, setOpen] = useState(false);
	const [text, setText] = useState("");
	const [errorMsg, setErrorMsg] = useState("");
	const [createClip, { isLoading }] = useCreateClipMutation();

	const handleClickOpen = () => {
		setOpen(true);
		setText("");
		setErrorMsg("");
	};

	const handleClose = () => setOpen(false);

	const handleClip = async () => {
		if (props.startEnd[1] - props.startEnd[0] > 20) {
			setErrorMsg("Clip duration cannot exceed 20 seconds.");
			return;
		}

		try {
			const response = await createClip({
				guild_id: props.params.guild_id ?? "",
				channel_id: props.params.channel_id ?? "",
				year: props.params.year ?? "",
				month: Number(props.params.month ?? ""),
				file_name: props.params.file_name ?? "",
				start: props.startEnd[0],
				end: props.startEnd[1],
				name: text.length > 0 ? text : undefined,
			}).unwrap();

			if (response && response.status === "success") {
				const fileRes = await authedFetch(
					`audio/clips/${props.params.guild_id}/${response.id}`,
				);
				if (!fileRes.ok) throw new Error(`download failed: ${fileRes.status}`);
				const blob = await fileRes.blob();
				const objectUrl = URL.createObjectURL(blob);
				const a = document.createElement("a");
				a.href = objectUrl;
				a.download = response.name
					? `${response.name}.ogg`
					: `${response.id}.ogg`;
				a.click();
				URL.revokeObjectURL(objectUrl);
			}

			setOpen(false);
		} catch (error) {
			console.error("Failed to create clip:", error);
		}
	};

	return (
		<>
			<Button
				variant="contained"
				onClick={handleClickOpen}
				disabled={props.disabled}
			>
				Clip
			</Button>
			<BaseDialog
				open={open}
				onClose={handleClose}
				error={errorMsg}
				busy={isLoading}
				actions={
					<>
						<Button onClick={handleClose} disabled={isLoading}>
							Cancel
						</Button>
						<Button onClick={handleClip} disabled={isLoading}>
							{isLoading ? "Creating..." : "Clip"}
						</Button>
					</>
				}
			>
				<DialogContentText>
					Enter a name for this clip. Will return an error if name is a
					duplicate. Leave blank for default name
				</DialogContentText>
				<TextField
					value={text}
					onChange={(e) => setText(e.currentTarget.value)}
					autoFocus
					margin="dense"
					id="name"
					label="Name"
					type="text"
					fullWidth
					variant="standard"
					autoComplete="off"
					disabled={isLoading}
				/>
			</BaseDialog>
		</>
	);
}
