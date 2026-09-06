import { ChevronDown as ExpandMoreIcon } from "lucide-react";
import type { ReactNode, RefObject } from "react";
import {
	Badge,
	Button,
	Disclosure,
	DisclosurePanel,
	DisclosureTrigger,
	Notice,
	Select,
	SelectItem,
} from "../../shared/ui";
import { formatDuration } from "../../utils/formatTime";
import type { PlaybackSegment } from "./logicalSessionTimeline";

export function LogicalSessionSummary(props: {
	sessionId: string;
	state: string;
	userId: string;
	startedAtMs: number;
	durationMs: number;
	physicalCount: number;
	currentSegment?: PlaybackSegment;
}) {
	const current = props.currentSegment;
	return (
		<>
			<div className="flex flex-wrap flex-row gap-2 mb-4">
				<Badge>{`Session ${props.sessionId}`}</Badge>
				<Badge tone={props.state === "active" ? "danger" : "neutral"}>
					{props.state}
				</Badge>
				<Badge>{`User ${props.userId}`}</Badge>
				<Badge appearance={"outline"}>{`${props.physicalCount} physical ${
					props.physicalCount === 1 ? "file" : "files"
				}`}</Badge>
				{current && (
					<Badge
						tone={
							current.reason === "channel_filtered"
								? "neutral"
								: current.kind === "silence"
									? "warning"
									: "accent"
						}
					>
						{current.reason === "channel_filtered"
							? `Channel ${current.channel_id ?? "?"} muted`
							: current.kind === "silence"
								? `Silence · ${current.reason ?? "gap"}`
								: `Channel ${current.channel_id ?? "?"}`}
					</Badge>
				)}
			</div>
			<p className="text-muted text-sm mb-2">
				Started {new Date(props.startedAtMs).toLocaleString()} · duration{" "}
				{formatDuration(props.durationMs / 1_000)}
			</p>
		</>
	);
}

export function PlaybackActionsPanel(props: { children: ReactNode }) {
	return (
		<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none p-4 my-4">
			{props.children}
		</div>
	);
}

export function SessionClipEditorPanel(props: {
	children: ReactNode;
	panelRef: RefObject<HTMLDivElement | null>;
}) {
	return (
		<div
			ref={props.panelRef}
			className="rounded-md border border-ui-border bg-surface text-fg shadow-sm p-4 my-4"
		>
			{props.children}
		</div>
	);
}

export function PhysicalRecordingsPanel(props: {
	sessionId: string;
	fragments: PlaybackSegment[];
	allFragments: PlaybackSegment[];
	channelIds: string[];
	effectiveChannelId: string | null;
	onSelectChannel: (channelId: string | null) => void;
	onSeek: (positionMs: number) => void;
}) {
	return (
		<Disclosure className="my-4">
			<DisclosureTrigger icon={<ExpandMoreIcon />}>
				<div className="flex items-center flex-wrap flex-row gap-2">
					<p className="leading-6">Physical recordings</p>
					<Badge
						appearance={"outline"}
						size={"sm"}
					>{`${props.fragments.length} ${
						props.fragments.length === 1 ? "file" : "files"
					}`}</Badge>
					{props.effectiveChannelId && (
						<Badge
							tone={"accent"}
							size={"sm"}
						>{`Channel ${props.effectiveChannelId} only`}</Badge>
					)}
				</div>
			</DisclosureTrigger>
			<DisclosurePanel>
				<div className="flex justify-between items-stretch min-[900px]:items-start flex-col min-[900px]:flex-row gap-4">
					<p className="text-muted text-sm">
						Session combines channel-bound files into one timestamp-aligned
						timeline.
					</p>
					{props.channelIds.length > 1 && (
						<div className="relative flex min-w-65">
							<Select
								label="Playback channel"
								selectedKey={props.effectiveChannelId ?? ""}
								onSelectionChange={(value) =>
									props.onSelectChannel(
										value === null || value === "" ? null : String(value),
									)
								}
							>
								<SelectItem id={""}>All channels</SelectItem>
								{props.channelIds.map((channelId) => {
									const count = props.allFragments.filter(
										(fragment) => fragment.channel_id === channelId,
									).length;
									return (
										<SelectItem key={channelId} id={channelId}>
											Channel {channelId} · {count}{" "}
											{count === 1 ? "file" : "files"}
										</SelectItem>
									);
								})}
							</Select>
						</div>
					)}
				</div>
				<div className="flex flex-col gap-1.5 mt-3">
					{props.fragments.map((fragment, index) => (
						<Button
							key={
								fragment.audio_file_id ??
								`${fragment.start_ms}-${fragment.end_ms}`
							}
							className="justify-start [text-transform:none] px-2"
							variant="ghost"
							onPress={() => props.onSeek(fragment.start_ms)}
						>
							<div className="text-left">
								<p className="text-sm">
									Fragment {(fragment.segment_index ?? index) + 1} · Channel{" "}
									{fragment.channel_id ?? "?"} ·{" "}
									{formatDuration(fragment.start_ms / 1_000)} –{" "}
									{formatDuration(fragment.end_ms / 1_000)}
								</p>
								<span className="text-muted text-xs leading-5">
									{fragment.file_name ?? `File ${fragment.audio_file_id}`}
								</span>
							</div>
						</Button>
					))}
				</div>
				{props.effectiveChannelId && (
					<Notice className="mt-3" tone={"info"} announce="status">
						Only Channel {props.effectiveChannelId} plays. Other channels stay
						muted while timeline offsets remain unchanged. Downloads and clips
						still use full session.
					</Notice>
				)}
			</DisclosurePanel>
		</Disclosure>
	);
}
