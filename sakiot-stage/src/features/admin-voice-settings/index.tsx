import { useState } from "react";
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
import { SavedValueChanged } from "../../shared/SavedValueChanged";
import {
	Button,
	Notice,
	Radio,
	RadioGroup,
	ScrollRegion,
	TextField,
} from "../../shared/ui";
import { sameIds, useDraftField } from "../../shared/useDraftField";
import { BotLogPanel } from "./BotLog";
import { KeepAllowancesPanel } from "./KeepAllowances";

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
	// Drafts survive refreshes (another admin saving, realtime updates);
	// untouched fields follow the saved value.
	const seconds = useDraftField(
		data ? String(data.pending_cap_seconds) : undefined,
		guildId,
	);
	const [validation, setValidation] = useState<string | null>(null);
	const {
		data: recordingPolicy,
		isError: recordingPolicyError,
		error: recordingPolicyLoadError,
		refetch: refetchRecordingPolicy,
	} = useGetGuildRecordingPolicyQuery(guildId, { skip: !guildId });
	const [saveRecordingPolicy, recordingSaveState] =
		useSetGuildRecordingPolicyMutation();
	const excludedChannels = useDraftField(
		recordingPolicy?.excluded_channel_ids,
		guildId,
		sameIds,
	);
	const noticeDelivery = useDraftField(
		recordingPolicy?.notice_delivery,
		guildId,
	);

	const handleRecordingPolicySave = async () => {
		try {
			await saveRecordingPolicy({
				guild_id: guildId,
				excluded_channel_ids: excludedChannels.value ?? [],
				notice_delivery: noticeDelivery.value ?? "dm",
			}).unwrap();
			excludedChannels.markSaved();
			noticeDelivery.markSaved();
		} catch {
			// recordingSaveState.isError renders the failure; drafts are kept.
		}
	};

	const handleSave = async () => {
		const parsed = Number(seconds.value);
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
		try {
			await save({ guild_id: guildId, pending_cap_seconds: parsed }).unwrap();
			seconds.markSaved();
		} catch {
			// saveState.isError renders the failure; the draft is kept.
		}
	};

	const handleReset = async () => {
		setValidation(null);
		try {
			await reset(guildId).unwrap();
			seconds.takeSaved();
		} catch {
			// resetState.isError renders the failure notice below; without this
			// catch the rejection was silent.
		}
	};

	if (!guildId) return <div className="p-4">Missing guild id.</div>;

	return (
		<div className="flex h-full min-h-0 flex-col">
			<h5 className="font-semibold tracking-tight text-2xl shrink-0 p-4 pb-2">
				Voice Settings
			</h5>
			<ScrollRegion fill aria-label="Voice settings">
				<div className="px-4 pb-4 max-w-190 space-y-5">
					<div className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-6">
						<h6 className="font-medium tracking-[0.001em] text-xl mb-2">
							Pending recording timeout
						</h6>
						<p className="leading-6 text-muted mb-4">
							Guilds without an AFK channel finalize users who do not follow the
							bot after this cap. Disconnect and AFK events still use a
							60-second grace. Guilds with an AFK channel have no absolute cap.
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
									value={seconds.value ?? ""}
									description={
										data.is_default
											? "Using six-hour default."
											: "Guild override active."
									}
									onChange={seconds.edit}
									min={MIN_PENDING_SECONDS}
									step={60}
								/>
								{seconds.savedUpdate && (
									<SavedValueChanged
										onTakeSaved={seconds.takeSaved}
										onSaveMine={() => void handleSave()}
										isSaving={saveState.isLoading}
									/>
								)}
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
							Recording privacy
						</h6>
						<p className="leading-6 text-muted mb-4">
							Excluded voice channels will not start new recordings.
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
												checked={(excludedChannels.value ?? []).includes(
													channel.id,
												)}
												onChange={(event) => {
													const checked = event.currentTarget.checked;
													const current = excludedChannels.value ?? [];
													excludedChannels.edit(
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
								<fieldset className="space-y-2">
									<legend className="font-medium">
										How members are told they are recorded
									</legend>
									<RadioGroup
										aria-label="How members are told they are recorded"
										value={noticeDelivery.value ?? "dm"}
										onChange={(value) =>
											noticeDelivery.edit(
												value === "channel" ? "channel" : "dm",
											)
										}
									>
										<Radio value="dm">Direct message</Radio>
										<Radio value="channel">
											Message in the voice channel's chat
										</Radio>
									</RadioGroup>
									<p className="text-muted">
										Each member is told once in this server, the first time they
										are recorded here. If this way fails, the bot tries the
										other.
									</p>
								</fieldset>
								{excludedChannels.savedUpdate && (
									<SavedValueChanged
										onTakeSaved={excludedChannels.takeSaved}
										onSaveMine={() => void handleRecordingPolicySave()}
										isSaving={recordingSaveState.isLoading}
									/>
								)}
								{noticeDelivery.savedUpdate && (
									<SavedValueChanged
										onTakeSaved={noticeDelivery.takeSaved}
										onSaveMine={() => void handleRecordingPolicySave()}
										isSaving={recordingSaveState.isLoading}
									/>
								)}
								<Button
									variant="primary"
									isDisabled={recordingSaveState.isLoading}
									onPress={handleRecordingPolicySave}
								>
									Save recording policy
								</Button>
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
					<KeepAllowancesPanel guildId={guildId} />
					<BotLogPanel guildId={guildId} />
				</div>
			</ScrollRegion>
		</div>
	);
}
