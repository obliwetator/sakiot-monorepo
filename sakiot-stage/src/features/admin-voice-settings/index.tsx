import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { managerActionFailure as failure } from "../../app/apiError";
import {
	useDeleteGuildVoiceSettingsMutation,
	useGetGuildRecordingPolicyQuery,
	useGetGuildVoiceSettingsQuery,
	useSetGuildRecordingPolicyMutation,
	useSetGuildVoiceSettingsMutation,
} from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import { Button, Notice, TextField } from "../../shared/ui";

const MIN_PENDING_SECONDS = 60;

export function GuildVoiceSettingsPage() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const guildId = guild_id ?? "";
	const {
		data,
		isLoading,
		isError,
		error: loadError,
		refetch,
	} = useGetGuildVoiceSettingsQuery(guildId, {
		skip: !guildId,
	});
	const [save, saveState] = useSetGuildVoiceSettingsMutation();
	const [reset, resetState] = useDeleteGuildVoiceSettingsMutation();
	const [seconds, setSeconds] = useState("21600");
	const [validation, setValidation] = useState<string | null>(null);
	const {
		data: recordingPolicy,
		isError: recordingPolicyError,
		error: recordingPolicyLoadError,
		refetch: refetchRecordingPolicy,
	} = useGetGuildRecordingPolicyQuery(guildId, { skip: !guildId });
	const [saveRecordingPolicy, recordingSaveState] =
		useSetGuildRecordingPolicyMutation();
	const [retentionDays, setRetentionDays] = useState("");
	const [excludedChannels, setExcludedChannels] = useState<string[]>([]);
	const [recordingValidation, setRecordingValidation] = useState<string | null>(
		null,
	);

	useEffect(() => {
		if (data) setSeconds(String(data.pending_cap_seconds));
	}, [data]);
	useEffect(() => {
		if (recordingPolicy) {
			setRetentionDays(recordingPolicy.retention_days?.toString() ?? "");
			setExcludedChannels(recordingPolicy.excluded_channel_ids);
		}
	}, [recordingPolicy]);

	const handleRecordingPolicySave = async () => {
		const days = retentionDays.trim() === "" ? null : Number(retentionDays);
		if (days !== null && (!Number.isInteger(days) || days < 1 || days > 3650)) {
			setRecordingValidation(
				"Retention must be blank or between 1 and 3650 days.",
			);
			return;
		}
		setRecordingValidation(null);
		await saveRecordingPolicy({
			guild_id: guildId,
			retention_days: days,
			excluded_channel_ids: excludedChannels,
		});
	};

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
		try {
			const restored = await reset(guildId).unwrap();
			setSeconds(String(restored.pending_cap_seconds));
		} catch {
			// resetState.isError renders the failure notice below; without this
			// catch the rejection was silent.
		}
	};

	if (!guildId) return <div className="p-4">Missing guild id.</div>;

	return (
		<div className="p-4 max-w-190 space-y-5">
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
					<LoadFailure
						error={loadError}
						what="Could not load voice settings."
						onRetry={() => void refetch()}
					/>
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
								{failure(saveState.error, "The override was not saved.")}
							</Notice>
						)}
						{resetState.isSuccess && (
							<Notice tone={"success"} announce="status">
								Default restored.
							</Notice>
						)}
						{resetState.isError && (
							<Notice tone={"error"} announce="alert">
								{failure(
									resetState.error,
									"The six-hour default was not restored.",
								)}
							</Notice>
						)}
					</div>
				)}
			</div>
			<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-6">
				<h6 className="font-medium tracking-[0.001em] text-xl mb-2">
					Recording privacy and retention
				</h6>
				<p className="leading-6 text-muted mb-4">
					Retention is off by default. When enabled, finalized recordings older
					than the chosen period are hidden from view, along with their clips.
					Media and metadata are retained. Excluded voice channels will not
					start new recordings.
				</p>
				{recordingPolicyError && (
					<LoadFailure
						error={recordingPolicyLoadError}
						what="Could not load the recording policy."
						onRetry={() => void refetchRecordingPolicy()}
					/>
				)}
				{recordingPolicy && (
					<div className="flex flex-col gap-4">
						<TextField
							label="Hide recordings after (days)"
							type="number"
							value={retentionDays}
							onChange={setRetentionDays}
							description="Leave blank to keep recordings indefinitely."
							min={1}
							max={3650}
						/>
						<fieldset className="space-y-2">
							<legend className="font-medium">
								Channels excluded from recording
							</legend>
							{recordingPolicy.channels.length === 0 && (
								<p className="text-muted">No voice channels found.</p>
							)}
							{recordingPolicy.channels.map((channel) => (
								<label key={channel.id} className="flex items-center gap-2">
									<input
										type="checkbox"
										checked={excludedChannels.includes(channel.id)}
										onChange={(event) => {
											const checked = event.currentTarget.checked;
											setExcludedChannels((current) =>
												checked
													? [...current, channel.id]
													: current.filter((id) => id !== channel.id),
											);
										}}
									/>
									<span>{channel.name}</span>
								</label>
							))}
						</fieldset>
						<Button
							variant="primary"
							isDisabled={recordingSaveState.isLoading}
							onPress={handleRecordingPolicySave}
						>
							Save recording policy
						</Button>
						{recordingValidation && (
							<Notice tone="warning" announce="status">
								{recordingValidation}
							</Notice>
						)}
						{recordingSaveState.isSuccess && (
							<Notice tone="success" announce="status">
								Recording policy saved.
							</Notice>
						)}
						{recordingSaveState.isError && (
							<Notice tone="error" announce="alert">
								{failure(
									recordingSaveState.error,
									"The recording policy was not saved.",
								)}
							</Notice>
						)}
					</div>
				)}
			</div>
		</div>
	);
}
