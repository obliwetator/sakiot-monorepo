import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useDeleteGuildVoiceSettingsMutation,
	useGetGuildVoiceSettingsQuery,
	useSetGuildVoiceSettingsMutation,
} from "../../app/apiSlice";
import {
	Button,
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Text,
	TextField,
} from "../../shared/ui";

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

	if (!guildId) {
		return (
			<Page>
				<Notice tone="error">Missing guild id.</Notice>
			</Page>
		);
	}

	return (
		<Page className="max-w-3xl space-y-5">
			<header className="space-y-1">
				<PageTitle>Voice Settings</PageTitle>
				<Text tone="muted">
					Manage voice recording rules and channel timeouts.
				</Text>
			</header>

			<Panel aria-labelledby="pending-timeout-heading" className="space-y-4">
				<div className="space-y-1">
					<SectionTitle id="pending-timeout-heading">
						Pending recording timeout
					</SectionTitle>
					<Text tone="muted">
						Guilds without an AFK channel finalize users who do not follow the
						bot after this cap. Disconnect and AFK events still use a 60-second
						grace. Guilds with an AFK channel have no absolute cap.
					</Text>
				</div>

				{isLoading && <Text aria-live="polite">Loading voice settings…</Text>}
				{isError && (
					<Notice tone="error" announce="alert">
						Could not load voice settings.
					</Notice>
				)}
				{data && (
					<form
						noValidate
						onSubmit={(event) => {
							event.preventDefault();
							void handleSave();
						}}
						className="space-y-4"
					>
						<TextField
							label="Pending cap (seconds)"
							name="pending-cap"
							type="number"
							min={MIN_PENDING_SECONDS}
							step={60}
							value={seconds}
							onChange={(value) => {
								setSeconds(value);
								setValidation(null);
								saveState.reset();
							}}
							description={
								data.is_default
									? "Using six-hour default."
									: "Guild override active."
							}
							error={validation ?? undefined}
						/>
						<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
							<Button type="submit" isPending={saveState.isLoading}>
								Save override
							</Button>
							<Button
								type="button"
								variant="secondary"
								onClick={handleReset}
								isDisabled={resetState.isLoading || data.is_default}
							>
								Restore six-hour default
							</Button>
						</div>
						{saveState.isSuccess && (
							<Notice tone="success" announce="status">
								Saved.
							</Notice>
						)}
						{saveState.isError && (
							<Notice tone="error" announce="alert">
								Save failed.
							</Notice>
						)}
						{resetState.isSuccess && (
							<Notice tone="success" announce="status">
								Default restored.
							</Notice>
						)}
					</form>
				)}
			</Panel>
		</Page>
	);
}
