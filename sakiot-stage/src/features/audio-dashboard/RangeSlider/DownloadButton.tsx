import type { Params } from "react-router-dom";
import { authedFetch } from "../../../app/authedFetch";
import type { AudioParams } from "../../../Constants";
import { saveBlob } from "../../../shared/download";
import { Button } from "../../../shared/ui";

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
			saveBlob(await fileRes.blob(), props.params.file_name ?? "");
		} catch (e) {
			console.error("Download failed", e);
		}
	};

	return (
		<Button variant="primary" onPress={handleDownload}>
			Download
		</Button>
	);
}
