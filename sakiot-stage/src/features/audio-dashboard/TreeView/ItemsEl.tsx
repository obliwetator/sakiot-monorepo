import type { IndividualFile } from "../../../Constants";
import { LivePill } from "./LiveDot";
import { parseFileName } from "./parseFileName";
import { StyledTreeItem } from "./StyledTreeItem";
import { recordingTreeItemId } from "./treeNavigation";

export function ItemsEl(props: {
	file: IndividualFile;
	year: number;
	month_name: number;
	isLive?: boolean;
}) {
	const { time, username: legacyUsername } = parseFileName(props.file.file);
	const username = props.file.display_name ?? legacyUsername;
	const title = props.file.channel_journey?.length
		? `${props.file.file} · channels ${props.file.channel_journey.join(" → ")}`
		: props.file.file;
	const access = props.file.access;
	const accessBadge =
		access === "visible-only" ? (
			<span
				className="ml-1.5 shrink-0 text-xs font-semibold text-amber-300"
				title="This role can see the channel but cannot join it — playback would be denied"
			>
				🔒 can't listen
			</span>
		) : access === "hidden" ? (
			<span
				className="ml-1.5 shrink-0 text-xs font-semibold text-red-400"
				title="This role cannot see the channel at all"
			>
				🚫 hidden
			</span>
		) : null;

	return (
		<StyledTreeItem
			itemId={recordingTreeItemId(props.file, props.year, props.month_name)}
			className={
				access === "hidden"
					? "opacity-40"
					: access === "visible-only"
						? "opacity-70"
						: undefined
			}
			label={
				<span
					className="flex w-full min-w-0 items-center justify-between gap-1 px-1 py-0.5 select-none text-sm"
					title={title}
				>
					<span className="min-w-0 truncate">
						<span className="font-mono">{time}</span>
						{username && <span className="ml-2">{username}</span>}
						{props.file.channel_journey &&
							props.file.channel_journey.length > 1 && (
								<span className="ml-2 text-xs opacity-75">
									{props.file.channel_journey.join(" → ")}
								</span>
							)}
					</span>
					<span className="flex shrink-0 items-center gap-1.5">
						{props.isLive && <LivePill />}
						{accessBadge}
					</span>
				</span>
			}
		/>
	);
}
