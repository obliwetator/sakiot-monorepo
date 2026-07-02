import Button from "@mui/material/Button";
import Typography from "@mui/material/Typography";
import { useState } from "react";
import { useParams } from "react-router-dom";
import { useJamItMutation } from "../../../app/apiSlice";
import type { UserGuilds } from "../../../Constants";
import { BaseDialog } from "../../../shared/BaseDialog";

export enum JamItRespStatus {
	CONNECTED,
	NOT_CONNECTED,
	UNKOWN,
	COOLDOWN,
}

export function JamIt(props: {
	disabled: boolean;
	userGuilds: UserGuilds[] | null;
}) {
	const [isError, setIsError] = useState<{
		type: JamItRespStatus;
		code: number;
		cooldownRemaining?: number;
	}>({
		type: JamItRespStatus.UNKOWN,
		code: 0,
	});
	const params = useParams();
	const [jamIt] = useJamItMutation();
	const [open, setOpen] = useState(false);

	if (!props.disabled) return null;

	const handleJamIt = async () => {
		try {
			const res = await jamIt({
				guild_id: params.guild_id as string,
				clip_name: params.file_name as string,
			}).unwrap();

			if (res.code) {
				setIsError({ type: JamItRespStatus.CONNECTED, code: res.code });
				setOpen(true);
			}
		} catch (err: unknown) {
			const apiError = err as {
				status?: number;
				data?: { code?: number; cooldown_remaining_seconds?: number };
			};
			if (apiError?.status === 429) {
				setIsError({
					type: JamItRespStatus.COOLDOWN,
					code: apiError?.data?.code ?? 3,
					cooldownRemaining: apiError?.data?.cooldown_remaining_seconds,
				});
			} else {
				setIsError({ type: JamItRespStatus.NOT_CONNECTED, code: 0 });
			}
			setOpen(true);
		}
	};

	const handleClose = () => {
		setOpen(false);
		setIsError({ type: JamItRespStatus.CONNECTED, code: 0 });
	};

	const shouldShow =
		isError.code > 0 ||
		isError.type === JamItRespStatus.NOT_CONNECTED ||
		isError.type === JamItRespStatus.COOLDOWN;

	return (
		<>
			<Button onClick={handleJamIt} variant="contained">
				Jam It
			</Button>
			{shouldShow && (
				<BaseDialog
					open={open}
					onClose={handleClose}
					title={
						isError.type === JamItRespStatus.COOLDOWN ? "On cooldown" : "Error"
					}
				>
					<Typography>
						{isError.type === JamItRespStatus.COOLDOWN ? (
							<>Try again in {isError.cooldownRemaining ?? "?"}s.</>
						) : (
							<>
								error code: {isError.code}
								<br />
								TODO: proper messages
								<br />
								number 0 = Success
								<br />
								number 1 = bot is not in voice channel
								<br />
								number &gt;= 2 =¯\_(ツ)_/¯
							</>
						)}
					</Typography>
				</BaseDialog>
			)}
		</>
	);
}
