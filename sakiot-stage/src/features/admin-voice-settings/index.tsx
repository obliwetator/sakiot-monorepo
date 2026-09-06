import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useDeleteGuildVoiceSettingsMutation,
	useGetGuildVoiceSettingsQuery,
	useSetGuildVoiceSettingsMutation,
} from "../../app/apiSlice";
import { Button, Notice, TextField } from "../../shared/ui";

const MIN_PENDING_SECONDS = 60;

export function GuildVoiceSettingsPage() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const guildId = guild_id ?? "";
	const { data, isLoading, isError } = useGetGuildVoiceSettingsQuery(guildId, {
		skip: !guildId,
	});
	const [save, saveState] = useSetGuildVoiceSettingsMutation();
	const [reset, resetState] = useDeleteGuildVoiceSettingsMutation();
	const [seconds, setSeconds] = useState("21600");
	const [validation, setValidation] = useState<string | null>(null);

	useEffect(() => {
		if (data) setSeconds(String(data.pending_cap_seconds));
	}, [data]);

	const handleSave = async () => {
		const parsed = Number(seconds);
		if (
			!Number.isInteger(parsed) ||
			!Number.isFinite(parsed) ||
			parsed < MIN_PENDING_SECONDS
		) {
			setValidation(
				`Pending timeout must be at least ${MIN_PENDING_SECONDS} seconds.`,
			);
			return;
		}
		setValidation(null);
		await save({ guild_id: guildId, pending_cap_seconds: parsed });
	};

	const handleReset = async () => {
		setValidation(null);
		const restored = await reset(guildId).unwrap();
		setSeconds(String(restored.pending_cap_seconds));
	};

	if (!guildId) return <div className="p-4">Missing guild id.</div>;

	return (
		<div className="p-4 max-w-190">
			<h5 className="font-semibold tracking-tight text-2xl mb-2">
				Voice Settings
			</h5>
			<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-6">
				<h6 className="font-medium tracking-[0.001em] text-xl mb-2">
					Pending recording timeout
				</h6>
				<p className="leading-6 text-muted mb-4">
					Guilds without an AFK channel finalize users who do not follow the bot
					after this cap. Disconnect and AFK events still use a 60-second grace.
					Guilds with an AFK channel have no absolute cap.
				</p>

				{isLoading && <p className="leading-6">Loading voice settings…</p>}
				{isError && (
					<Notice tone={"error"} announce="alert">
						Could not load voice settings.
					</Notice>
				)}
				{data && (
					<div className="flex flex-col gap-4">
						<TextField
							label="Pending cap (seconds)"
							type="number"
							value={seconds}
							description={
								data.is_default
									? "Using six-hour default."
									: "Guild override active."
							}
							onChange={(value) => setSeconds(value)}
							min={MIN_PENDING_SECONDS}
							step={60}
						/>
						<div className="flex flex-col min-[600px]:flex-row gap-2">
							<Button
								variant="primary"
								isDisabled={saveState.isLoading}
								onPress={handleSave}
							>
								Save override
							</Button>
							<Button
								variant="outline"
								isDisabled={resetState.isLoading || data.is_default}
								onPress={handleReset}
							>
								Restore six-hour default
							</Button>
						</div>
						{validation && (
							<Notice tone={"warning"} announce="status">
								{validation}
							</Notice>
						)}
						{saveState.isSuccess && (
							<Notice tone={"success"} announce="status">
								Saved.
							</Notice>
						)}
						{saveState.isError && (
							<Notice tone={"error"} announce="alert">
								Save failed.
							</Notice>
						)}
						{resetState.isSuccess && (
							<Notice tone={"success"} announce="status">
								Default restored.
							</Notice>
						)}
					</div>
				)}
			</div>
		</div>
	);
}
