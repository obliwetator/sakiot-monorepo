import {
	Scissors as ContentCutIcon,
	ChevronDown as ExpandMoreIcon,
	Film as MovieIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import {
	type ClipData,
	useDeleteClipMutation,
	useGetAuthDetailsQuery,
	useGetClipsQuery,
} from "../../app/apiSlice";
import { isLoggedIn as hasLoggedInCookie } from "../../app/authedFetch";
import { useAppSelector } from "../../app/hooks";
import { useAsRole } from "../../app/useAsRole";
import { PATH_PREFIX_FOR_LOGGED_USERS, type UserGuilds } from "../../Constants";
import { canDeleteClip } from "../../shared/permissions";
import {
	Button,
	cn,
	DialogHeading,
	Disclosure,
	DisclosurePanel,
	DisclosureTrigger,
	Drawer,
	Modal,
	Tab,
	TabList,
	TabPanel,
	Tabs,
	useMediaQuery,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import { ViewAsRoleBanner } from "../members/ViewAsRoleBanner";
import { ClipPlayer } from "./ClipPlayer";
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

	const elements = props.data.map((el, index) => {
		return (
			<Disclosure
				key={el.clip_id}
				className={cn(
					"mb-2 border [border-radius:8px_!important] [box-shadow:none] before:hidden",
					location.pathname.endsWith(encodeURIComponent(el.clip_id))
						? "border-creative"
						: "border-ui-border",
				)}
				isExpanded={expanded === `panel${index}`}
				onExpandedChange={handleChange(`panel${index}`)}
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
				<DisclosurePanel>
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
	};
	const params = useParams();

	const [deleteClip] = useDeleteClipMutation();

	const handleYes = async () => {
		if (params.guild_id) {
			try {
				await deleteClip({
					guild_id: params.guild_id,
					file_name: props.clip_id,
				}).unwrap();
				setOpen(false);
			} catch (error) {
				console.error("Failed to delete clip:", error);
				setOpen(false);
			}
		} else {
			setOpen(false);
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
			<Modal
				aria-labelledby="alert-dialog-title"
				aria-describedby="alert-dialog-description"
				isOpen={open}
				onOpenChange={(isOpen) => {
					if (!isOpen) handleClose();
				}}
			>
				<DialogHeading id="alert-dialog-title">
					{"Confirm deletion?"}
				</DialogHeading>
				<div className="space-y-3 px-5 py-4">
					<p
						id="alert-dialog-description"
						className="text-sm leading-6 text-slate-200"
					>
						Are you sure you want to delete the clip?
					</p>
				</div>
				<div className="flex justify-end gap-2 border-t border-ui-border px-5 py-3">
					<Button onPress={handleClose}>No</Button>
					<Button autoFocus onPress={handleYes}>
						YEP
					</Button>
				</div>
			</Modal>
		</div>
	);
}

function clipAbsoluteStartMs(clip: ClipData | null): number | null {
	if (!clip?.original_file_name) return null;
	const ts = Number.parseInt(clip.original_file_name.split("-")[0] ?? "", 10);
	if (!Number.isFinite(ts)) return null;
	return ts + clip.start_time * 1000;
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
	const { data, isError, isSuccess } = useGetClipsQuery(
		{ guild_id: guildId, ...asRoleArg },
		{
			skip: !guildId,
			refetchOnMountOrArgChange: true,
		},
	);

	if (isError) {
		console.error("cannot get clip data");
	}

	if (isSuccess && data) {
		return (
			<div className="p-3 min-[900px]:p-6">
				<ViewAsRoleBanner guildId={guildId} />
				<ClipsLayout
					data={data}
					params={params}
					currentUserId={authData?.user?.user_id ?? null}
					guildSelected={guild}
				/>
			</div>
		);
	} else {
		return <div>No clip data</div>;
	}
}

function ClipsLayout(props: {
	data: ClipData[];
	params: { guild_id?: string; file_name?: string };
	currentUserId: string | null;
	guildSelected: UserGuilds | null;
}) {
	const isDesktop = useMediaQuery("(min-width: 900px)");
	const [drawerOpen, setDrawerOpen] = useState(false);
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

	const composedClips = props.data.filter(isComposedClip);
	const pureClips = props.data.filter((clip) => !isComposedClip(clip));

	const list = (
		<>
			<div className="p-2">
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
			<Tabs
				className="border-b border-ui-border"
				selectedKey={clipTab}
				onSelectionChange={(value) => {
					if (value === "clips" || value === "combined") setClipTab(value);
				}}
			>
				<TabList aria-label="View">
					<Tab id={"clips"}>Clips</Tab>
					<Tab
						id={"combined"}
					>{`Combined${composedClips.length > 0 ? ` (${composedClips.length})` : ""}`}</Tab>
				</TabList>
				<TabPanel id="clips">
					<ClipList
						onSelect={() => setDrawerOpen(false)}
						data={pureClips}
						currentUserId={props.currentUserId}
						guildSelected={props.guildSelected}
					/>
				</TabPanel>
				<TabPanel id="combined">
					{composedClips.length === 0 && (
						<p className="text-muted text-sm p-4">
							No combined clips yet. Export a composition from the clip editor.
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
	const absoluteStartMs = clipAbsoluteStartMs(selectedClip ?? null);

	return (
		<div className="flex flex-col min-[900px]:flex-row w-full gap-2">
			{isDesktop ? (
				<div className="[flex:0_0_34%] max-w-120 w-full overflow-auto p-2">
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
						<div className="w-80">{list}</div>
					</Drawer>
				</div>
			)}
			<div className="flex-1 min-w-0">
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
