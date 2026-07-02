import Button from "@mui/material/Button";
import type { Params } from "react-router-dom";
import { authedFetch } from "../../../app/authedFetch";
import type { AudioParams } from "../../../Constants";

export function DownloadButton(props: {
	isClip: boolean;
	isSilence: boolean;
	params: Readonly<Params<AudioParams>>;
}) {
	const handleDownload = async () => {
		const url = props.isClip
			? `audio/clips/${props.params.guild_id}/${props.params.file_name}`
			: `download/${props.params.guild_id}/${props.params.channel_id}/${props.params.year}/${props.params.month}/${props.params.file_name}.ogg${props.isSilence ? "?silence=true" : ""}`;
		try {
			const fileRes = await authedFetch(url);
			if (!fileRes.ok) throw new Error(`download failed: ${fileRes.status}`);
			const blob = await fileRes.blob();
			const objectUrl = URL.createObjectURL(blob);
			const a = document.createElement("a");
			a.href = objectUrl;
			a.download = props.params.file_name ?? "";
			a.click();
			URL.revokeObjectURL(objectUrl);
		} catch (e) {
			console.error("Download failed", e);
		}
	};

	return (
		<Button variant="contained" onClick={handleDownload}>
			Download
		</Button>
	);
}
