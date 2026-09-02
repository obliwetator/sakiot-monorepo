import { Trash2 } from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useDeleteUserOverrideMutation,
	useGetGuildCooldownQuery,
	useListUserOverridesQuery,
	useSetGuildCooldownMutation,
	useSetUserOverrideMutation,
} from "../../app/apiSlice";
import {
	Button,
	IconButton,
	Notice,
	Page,
	PageTitle,
	Panel,
	SectionTitle,
	Table,
	TableBody,
	TableCell,
	TableContainer,
	TableHead,
	TableHeader,
	TableRow,
	Text,
	TextField,
} from "../../shared/ui";

function parseSeconds(value: string): number | null {
	const seconds = Number(value);
	return Number.isFinite(seconds) && Number.isInteger(seconds) && seconds >= 0
		? seconds
		: null;
}

export function GuildAdminCooldowns() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";

	const { data: guildCooldown, isLoading: loadingGuild } =
		useGetGuildCooldownQuery(gid, { skip: !gid });
	const { data: overrides, isLoading: loadingOverrides } =
		useListUserOverridesQuery(gid, { skip: !gid });
	const [setGuildCooldown, setGuildState] = useSetGuildCooldownMutation();
	const [setUserOverride, setOverrideState] = useSetUserOverrideMutation();
	const [deleteUserOverride, deleteState] = useDeleteUserOverrideMutation();

	const [guildSeconds, setGuildSeconds] = useState("0");
	const [newUserId, setNewUserId] = useState("");
	const [newSeconds, setNewSeconds] = useState("0");
	const [guildError, setGuildError] = useState<string | null>(null);
	const [overrideError, setOverrideError] = useState<string | null>(null);
	const [deletingUserId, setDeletingUserId] = useState<string | null>(null);
	const [deleteFeedback, setDeleteFeedback] = useState<{
		tone: "success" | "error";
		message: string;
	} | null>(null);
	const [initializedGuildId, setInitializedGuildId] = useState<string | null>(
		null,
	);
	const guildInputDirtyRef = useRef(false);

	useEffect(() => {
		if (
			guildCooldown &&
			initializedGuildId !== gid &&
			!guildInputDirtyRef.current
		) {
			setGuildSeconds(String(guildCooldown.cooldown_seconds));
			setInitializedGuildId(gid);
		}
	}, [gid, guildCooldown, initializedGuildId]);

	const prevGidRef = useRef(gid);
	// Reset the edit guard when navigation changes guilds; fetched data must not
	// overwrite a value the user started entering while the request was pending.
	useEffect(() => {
		if (prevGidRef.current !== gid) {
			prevGidRef.current = gid;
			guildInputDirtyRef.current = false;
			setInitializedGuildId(null);
		}
	}, [gid]);

	const handleSaveGuild = async (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		const seconds = parseSeconds(guildSeconds);
		if (seconds === null) {
			setGuildError("Cooldown must be a non-negative integer.");
			return;
		}

		setGuildError(null);
		setGuildState.reset();
		try {
			await setGuildCooldown({
				guild_id: gid,
				cooldown_seconds: seconds,
			}).unwrap();
		} catch {
			// RTK Query exposes the server failure through setGuildState below.
		}
	};

	const handleAddOverride = async (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		const seconds = parseSeconds(newSeconds);
		if (!newUserId.trim() || seconds === null) {
			setOverrideError("Provide a user id and non-negative integer seconds.");
			return;
		}

		setOverrideError(null);
		setOverrideState.reset();
		try {
			await setUserOverride({
				guild_id: gid,
				user_id: newUserId.trim(),
				cooldown_seconds: seconds,
			}).unwrap();
			setNewUserId("");
			setNewSeconds("0");
		} catch {
			// Keep the submitted values available so the user can retry.
		}
	};

	const handleDelete = async (userId: number) => {
		const userIdString = String(userId);
		setDeletingUserId(userIdString);
		setDeleteFeedback(null);
		deleteState.reset();
		try {
			await deleteUserOverride({
				guild_id: gid,
				user_id: userIdString,
			}).unwrap();
			setDeleteFeedback({
				tone: "success",
				message: `Override for user ${userIdString} deleted.`,
			});
		} catch {
			setDeleteFeedback({
				tone: "error",
				message: `Could not delete the override for user ${userIdString}.`,
			});
		} finally {
			setDeletingUserId(null);
		}
	};

	if (!gid) {
		return (
			<Page>
				<Notice tone="error">Missing guild id.</Notice>
			</Page>
		);
	}

	return (
		<Page className="space-y-5">
			<header className="space-y-1">
				<PageTitle>Jam cooldowns</PageTitle>
				<Text tone="muted">
					Control how often members can send a clip to the voice channel.
				</Text>
			</header>

			<Panel aria-labelledby="guild-default-heading" className="space-y-4">
				<div className="space-y-1">
					<SectionTitle id="guild-default-heading">Guild default</SectionTitle>
					<Text tone="muted">0 disables the cooldown for this guild.</Text>
				</div>

				{loadingGuild || (guildCooldown && initializedGuildId !== gid) ? (
					<Text aria-live="polite">Loading guild cooldown…</Text>
				) : null}

				<form
					noValidate
					onSubmit={handleSaveGuild}
					className="flex flex-col items-stretch gap-3 sm:flex-row sm:items-end"
				>
					<TextField
						label="Cooldown (seconds)"
						name="guild-cooldown"
						type="number"
						min={0}
						step={1}
						value={guildSeconds}
						onFocus={() => {
							if (initializedGuildId !== gid) {
								guildInputDirtyRef.current = true;
							}
						}}
						onChange={(value) => {
							guildInputDirtyRef.current = true;
							setGuildSeconds(value);
							setGuildError(null);
							setGuildState.reset();
						}}
						error={guildError ?? undefined}
						className="sm:w-64"
					/>
					<Button
						type="submit"
						variant="contained"
						isPending={setGuildState.isLoading}
					>
						Save default
					</Button>
				</form>

				{guildError && (
					<Notice tone="warning" announce="alert">
						{guildError}
					</Notice>
				)}
				{setGuildState.isSuccess && (
					<Notice tone="success" announce="status">
						Guild default saved.
					</Notice>
				)}
				{setGuildState.isError && (
					<Notice tone="error" announce="alert">
						Could not save the guild default.
					</Notice>
				)}
			</Panel>

			<Panel aria-labelledby="user-overrides-heading" className="space-y-4">
				<div className="space-y-1">
					<SectionTitle id="user-overrides-heading">
						Per-user overrides
					</SectionTitle>
					<Text tone="muted">
						Add a new override or update an existing member by user ID.
					</Text>
				</div>

				<form
					noValidate
					onSubmit={handleAddOverride}
					className="grid items-end gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,14rem)_auto]"
				>
					<TextField
						label="User ID"
						name="user-id"
						value={newUserId}
						onChange={(value) => {
							setNewUserId(value);
							setOverrideError(null);
							setOverrideState.reset();
						}}
						error={
							overrideError && !newUserId.trim() ? overrideError : undefined
						}
					/>
					<TextField
						label="Cooldown (seconds)"
						name="user-cooldown"
						type="number"
						min={0}
						step={1}
						value={newSeconds}
						onChange={(value) => {
							setNewSeconds(value);
							setOverrideError(null);
							setOverrideState.reset();
						}}
						error={
							overrideError && parseSeconds(newSeconds) === null
								? overrideError
								: undefined
						}
					/>
					<Button
						type="submit"
						variant="contained"
						isPending={setOverrideState.isLoading}
					>
						Add / Update
					</Button>
				</form>

				{overrideError && (
					<Notice tone="warning" announce="alert">
						{overrideError}
					</Notice>
				)}
				{setOverrideState.isSuccess && (
					<Notice tone="success" announce="status">
						User override saved.
					</Notice>
				)}
				{setOverrideState.isError && (
					<Notice tone="error" announce="alert">
						Could not save the user override.
					</Notice>
				)}
				{deleteFeedback && (
					<Notice
						tone={deleteFeedback.tone}
						announce={deleteFeedback.tone === "error" ? "alert" : "status"}
					>
						{deleteFeedback.message}
					</Notice>
				)}

				{loadingOverrides ? (
					<Text aria-live="polite">Loading admin cooldowns…</Text>
				) : (
					<TableContainer className="rounded-lg border border-slate-800">
						<Table>
							<caption className="sr-only">Per-user cooldown overrides</caption>
							<TableHeader>
								<TableRow>
									<TableHead scope="col">User ID</TableHead>
									<TableHead scope="col" className="text-right">
										Cooldown (s)
									</TableHead>
									<TableHead scope="col">Updated</TableHead>
									<TableHead scope="col" className="text-right">
										Actions
									</TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{(overrides ?? []).map((override) => {
									const userId = String(override.user_id);
									return (
										<TableRow key={override.user_id}>
											<TableCell className="font-mono text-xs text-cyan-100">
												{userId}
											</TableCell>
											<TableCell className="text-right font-medium">
												{override.cooldown_seconds}
											</TableCell>
											<TableCell>
												<time dateTime={override.updated_at}>
													{new Date(override.updated_at).toLocaleString()}
												</time>
											</TableCell>
											<TableCell className="text-right">
												<IconButton
													label={`Delete override for user ${userId}`}
													variant="danger"
													isPending={
														deleteState.isLoading && deletingUserId === userId
													}
													isDisabled={
														deleteState.isLoading && deletingUserId !== userId
													}
													onPress={() => handleDelete(override.user_id)}
												>
													<Trash2 aria-hidden="true" className="size-4" />
												</IconButton>
											</TableCell>
										</TableRow>
									);
								})}
								{(!overrides || overrides.length === 0) && (
									<TableRow>
										<TableCell
											colSpan={4}
											className="py-8 text-center text-muted"
										>
											No per-user overrides.
										</TableCell>
									</TableRow>
								)}
							</TableBody>
						</Table>
					</TableContainer>
				)}
			</Panel>
		</Page>
	);
}
