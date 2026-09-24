import {
	Scissors as ContentCutIcon,
	ChevronDown as ExpandMoreIcon,
	Film as MovieIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { problemFromError } from "../../app/apiError";
import {
	type ClipData,
	useDeleteClipMutation,
	useGetAuthDetailsQuery,
	useGetClipsQuery,
	useGetSessionManifestQuery,
} from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import { useAsRole } from "../../app/useAsRole";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../../Constants";
import { BaseDialog } from "../../shared/BaseDialog";
import { LoadFailure } from "../../shared/LoadFailure";
import { canDeleteClip } from "../../shared/permissions";
import {
	Button,
	cn,
	Disclosure,
	DisclosurePanel,
	DisclosureTrigger,
	Drawer,
	Notice,
	SearchInput,
	Tab,
	TabList,
	TabPanel,
	Tabs,
	useMediaQuery,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { ClipPlayer } from "./ClipPlayer";
import { filterClips } from "./clipSearch";
import { clipAbsoluteStartMs } from "./clipStart";
import { isComposedClip } from "./composedClip";

function ClipList(props: {
	onSelect: () => void;
	data: ClipData[];
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const navigate = useNavigate();
	const location = useLocation();
	const [expanded, setExpanded] = useState<string | false>(false);

	const handleClickAccordion = (guild_id: string, clip_id: string) => {
		const clipPath = `${PATH_PREFIX_FOR_LOGGED_USERS}/${guild_id}/clips/${encodeURIComponent(clip_id)}`;
		if (location.pathname !== clipPath) {
			navigate(clipPath);
		}
		props.onSelect();
	};

	const handleChange = (panel: string) => (isExpanded: boolean) => {
		setExpanded(isExpanded ? panel : false);
	};

	const elements = props.data.map((el) => {
		return (
			<Disclosure
				key={el.clip_id}
				className={cn(
					"mb-2 border rounded-[8px]! shadow-none before:hidden",
					location.pathname.endsWith(encodeURIComponent(el.clip_id))
						? "border-creative"
						: "border-ui-border",
				)}
				isExpanded={expanded === el.clip_id}
				onExpandedChange={handleChange(el.clip_id)}
			>
				<DisclosureTrigger
					icon={<ExpandMoreIcon />}
					onPress={() => handleClickAccordion(el.guild_id, el.clip_id)}
				>
					<div className="min-w-0 flex-1">
						<p className="leading-6 [overflow-wrap:anywhere]">
							{el.name || "Unnamed clip"}
						</p>
						<span className="text-muted text-xs leading-5">
							{formatDuration(el.length ?? 0)} · User {el.user_id}
						</span>
					</div>
				</DisclosureTrigger>
				{expanded === el.clip_id && (
					<DisclosurePanel data-testid={`clip-details-${el.clip_id}`}>
						<div className="flex flex-col gap-1">
							{isComposedClip(el) ? (
								<span className="text-muted text-xs leading-5">
									Composed in the clip editor
								</span>
							) : (
								<>
									<p className="text-sm">
										Channel {el.channel_id} · source offset{" "}
										{formatDuration(el.start_time)}
									</p>
									<span className="text-muted text-xs leading-5 [overflow-wrap:anywhere]">
										{el.original_file_name || "Unknown source recording"}
									</span>
								</>
							)}
							<AlertDialog
								clip_id={el.clip_id}
								canDelete={canDeleteClip(
									props.guildSelected,
									props.currentUserId,
									el.user_id,
								)}
							/>
						</div>
					</DisclosurePanel>
				)}
			</Disclosure>
		);
	});
	return <div>{elements}</div>;
}

function AlertDialog(props: { clip_id: string; canDelete: boolean }) {
	const [open, setOpen] = useState(false);

	const handleClickOpen = () => {
		if (!props.canDelete) return;
		setOpen(true);
	};

	const handleClose = () => {
		setOpen(false);
		setError(null);
	};
	const params = useParams();

	const [deleteClip, deleteState] = useDeleteClipMutation();
	const [error, setError] = useState<string | null>(null);

	const handleDelete = async () => {
		if (!params.guild_id) {
			setOpen(false);
			return;
		}
		setError(null);
		try {
			await deleteClip({
				guild_id: params.guild_id,
				file_name: props.clip_id,
			}).unwrap();
			setOpen(false);
		} catch (failure) {
			// Keep the dialog open so the failure is visible and can be retried.
			const problem = problemFromError(failure);
			setError(
				problem.kind === "clip_not_found"
					? "This clip no longer exists. It may already have been deleted."
					: `The clip was not deleted. ${problem.message}`,
			);
		}
	};

	return (
		<div>
			<Button
				variant="danger"
				isDisabled={!props.canDelete}
				onPress={handleClickOpen}
			>
				Delete
			</Button>
			<BaseDialog
				open={open}
				onClose={handleClose}
				title="Confirm deletion?"
				error={error ?? undefined}
				confirmLabel={error ? "Try again" : "Delete clip"}
				confirmVariant="danger"
				confirmDisabled={deleteState.isLoading}
				onConfirm={handleDelete}
				autoFocusCancel
			>
				<p className="text-sm leading-6 text-slate-200">
					Are you sure you want to delete the clip?
				</p>
			</BaseDialog>
		</div>
	);
}

export default function Clips() {
	const params = useParams();

	const guildSelected = useAppSelector((state) => state.app.guildSelected);
	const { data: authData } = useGetAuthDetailsQuery(undefined, {
		skip: !hasLoggedInCookie(),
	});
	const guildId = params.guild_id ?? "";
	const guild =
		authData?.guilds?.find((g) => g.id === guildId) ?? guildSelected;
	const { asRoleArg } = useAsRole();
	const {
		currentData: data,
		isError,
		error,
		refetch,
		isLoading,
		isFetching,
		isUninitialized,
	} = useGetClipsQuery(
		{ guild_id: guildId, ...asRoleArg },
		{
			skip: !guildId,
		},
	);

	// A failed or in-flight request must not read as "no clips": rendering the
	// same empty page for all three made a broken API look like an empty library.
	if (isError) {
		return (
			<div className="p-3 min-[900px]:p-6">
				<ViewAsRoleBanner guildId={guildId} />
				<LoadFailure
					error={error}
					what="Could not load clips."
					onRetry={() => void refetch()}
				/>
			</div>
		);
	}

	if (isLoading || isUninitialized || (!data && isFetching)) {
		return (
			<div className="p-3 min-[900px]:p-6">
				<ViewAsRoleBanner guildId={guildId} />
				<Notice announce="status">Loading clips…</Notice>
			</div>
		);
	}

	return (
		<div className="p-3 min-[900px]:p-6 h-full flex flex-col">
			<ViewAsRoleBanner guildId={guildId} />
			<ClipsLayout
				data={data ?? []}
				params={params}
				currentUserId={authData?.user?.user_id ?? null}
				guildSelected={guild}
			/>
		</div>
	);
}

function ClipsLayout(props: {
	data: ClipData[];
	params: { guild_id?: string; file_name?: string };
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const isDesktop = useMediaQuery("(min-width: 900px)");
	const [drawerOpen, setDrawerOpen] = useState(false);
	const [searchQuery, setSearchQuery] = useState("");
	const navigate = useNavigate();
	const [clipTab, setClipTab] = useState<"clips" | "combined">(() => {
		const selectedClipId = props.params.file_name
			? decodeURIComponent(props.params.file_name)
			: null;
		const selected = selectedClipId
			? props.data.find((c) => c.clip_id === selectedClipId)
			: null;
		return selected && isComposedClip(selected) ? "combined" : "clips";
	});

	useEffect(() => {
		if (!isDesktop) setDrawerOpen(false);
	}, [isDesktop]);

	const matchingClips = filterClips(props.data, searchQuery);
	const composedClips = matchingClips.filter(isComposedClip);
	const pureClips = matchingClips.filter((clip) => !isComposedClip(clip));
	const isSearching = searchQuery.trim().length > 0;

	const list = (
		<>
			<div className="p-2 pb-0 shrink-0">
				<SearchInput
					id="clip-search"
					label="Search clips"
					placeholder="Search clips..."
					value={searchQuery}
					onChange={setSearchQuery}
				/>
			</div>
			<div className="p-2 shrink-0">
				<Button
					className="w-full"
					variant="primary"
					onPress={() =>
						navigate(
							`${PATH_PREFIX_FOR_LOGGED_USERS}/${props.params.guild_id}/clips/editor`,
						)
					}
				>
					<ContentCutIcon />
					Clip editor
				</Button>
			</div>
			{/* The editor link and the tab strip stay put; only the rows below
			    them scroll, so the selected tab is always visible. */}
			<Tabs
				className="border-b border-ui-border flex flex-col flex-1 min-h-0"
				selectedKey={clipTab}
				onSelectionChange={(value) => {
					if (value === "clips" || value === "combined") setClipTab(value);
				}}
			>
				<TabList aria-label="View" className="shrink-0">
					<Tab id={"clips"}>Clips</Tab>
					<Tab
						id={"combined"}
					>{`Combined${composedClips.length > 0 ? ` (${composedClips.length})` : ""}`}</Tab>
				</TabList>
				<TabPanel id="clips" className="overflow-auto min-h-0 flex-1">
					{pureClips.length === 0 && (
						<p className="text-muted text-sm p-4">
							{isSearching
								? "No clips match this search."
								: "No clips yet. Cut one from the audio dashboard or the clip editor."}
						</p>
					)}
					<ClipList
						onSelect={() => setDrawerOpen(false)}
						data={pureClips}
						currentUserId={props.currentUserId}
						guildSelected={props.guildSelected}
					/>
				</TabPanel>
				<TabPanel id="combined" className="overflow-auto min-h-0 flex-1">
					{composedClips.length === 0 && (
						<p className="text-muted text-sm p-4">
							{isSearching
								? "No clips match this search."
								: "No combined clips yet. Export a composition from the clip editor."}
						</p>
					)}
					<ClipList
						onSelect={() => setDrawerOpen(false)}
						data={composedClips}
						currentUserId={props.currentUserId}
						guildSelected={props.guildSelected}
					/>
				</TabPanel>
			</Tabs>
		</>
	);

	const selectedClipId = props.params.file_name
		? decodeURIComponent(props.params.file_name)
		: null;
	const selectedClip = selectedClipId
		? props.data.find((c) => c.clip_id === selectedClipId)
		: null;
	// Session-derived clips have no timestamp in their source name, so their
	// absolute time comes from the session the clip was cut from.
	// currentData, not data: RTK keeps the previous argument's result while the
	// next one loads, which would compute this clip's start from another
	// session's timestamp.
	const { currentData: selectedSession } = useGetSessionManifestQuery(
		selectedClip?.recording_session_id ?? "",
		{ skip: !selectedClip?.recording_session_id },
	);
	const absoluteStartMs = clipAbsoluteStartMs(
		selectedClip ?? null,
		selectedSession?.started_at_ms ?? null,
	);

	// Confined to the outlet's height: the selection pane scrolls on its own
	// instead of stretching the page past the viewport.
	return (
		<div className="flex flex-col min-[900px]:flex-row w-full gap-2 flex-1 min-h-0">
			{isDesktop ? (
				<div className="[flex:0_0_34%] max-w-120 w-full p-2 min-h-0 flex flex-col overflow-hidden">
					{list}
				</div>
			) : (
				<div className="p-2">
					<Button
						className="w-full"
						variant="outline"
						onPress={() => setDrawerOpen(true)}
					>
						<MovieIcon />
						Browse clips
					</Button>
					<p className="text-muted text-sm mt-2 px-1 [word-break:break-word]">
						{selectedClip
							? `Current: ${selectedClip.name}`
							: "No clip selected"}
					</p>
					<Drawer
						isOpen={drawerOpen}
						side={"left"}
						onOpenChange={(isOpen) => {
							if (!isOpen) setDrawerOpen(false);
						}}
					>
						<div className="w-80 h-dvh flex flex-col">{list}</div>
					</Drawer>
				</div>
			)}
			<div className="flex-1 min-w-0 min-h-0 overflow-auto">
				{selectedClip && (
					<ClipPlayer
						key={selectedClip.clip_id}
						clip={selectedClip}
						absoluteStartMs={absoluteStartMs}
						canRename={canDeleteClip(
							props.guildSelected,
							props.currentUserId,
							selectedClip.user_id,
						)}
					/>
				)}
			</div>
		</div>
	);
}
