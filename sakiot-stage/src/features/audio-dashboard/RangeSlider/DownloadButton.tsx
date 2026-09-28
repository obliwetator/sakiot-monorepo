import { useState } from "react";
import type { Params } from "react-router-dom";
import { problemFromError } from "../../../app/apiError";
import { downloadFile } from "../../../app/download";
import type { AudioParams } from "../../../Constants";
import { Button, Notice } from "../../../shared/ui";

export function DownloadButton(props: {
	isClip: boolean;
	isSilence: boolean;
	params: Readonly<Params<AudioParams>>;
}) {
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const handleDownload = async () => {
		const url = props.isClip
			? `audio/clips/${props.params.guild_id}/${props.params.file_name}`
			: `download/${props.params.guild_id}/${props.params.channel_id}/${props.params.year}/${props.params.month}/${props.params.file_name}.ogg${props.isSilence ? "?silence=true" : ""}`;
		const what = props.isClip ? "clip" : "recording";
		setBusy(true);
		setError(null);
		try {
			await downloadFile(url, props.params.file_name ?? "");
		} catch (failure) {
			setError(
				`The ${what} download failed. ${problemFromError(failure).message}`,
			);
		} finally {
			setBusy(false);
		}
	};

	return (
		<>
			<Button variant="primary" isDisabled={busy} onPress={handleDownload}>
				{busy ? "Downloading…" : "Download"}
			</Button>
			{error && (
				<Notice tone="error" announce="alert">
					{error}
				</Notice>
			)}
		</>
	);
}
