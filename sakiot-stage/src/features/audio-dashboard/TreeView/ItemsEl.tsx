import { treeItemClasses } from "@mui/x-tree-view";
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

	return (
		<StyledTreeItem
			itemId={recordingTreeItemId(props.file, props.year, props.month_name)}
			className="bg-violet-600 overflow-hidden"
			sx={{
				[`& > .${treeItemClasses.content}`]: {
					borderBottom: "1px solid rgb(76 29 149)",
					cursor: "pointer",
				},
				[`& > .${treeItemClasses.content}.${treeItemClasses.selected}`]: {
					boxShadow: "inset 0 0 0 2px white",
				},
			}}
			label={
				<span
					className="block w-full px-2 py-1 select-none text-sm"
					title={title}
				>
					<span className="font-mono">{time}</span>
					{username && <span className="ml-2">{username}</span>}
					{props.file.channel_journey &&
						props.file.channel_journey.length > 1 && (
							<span className="ml-2 text-xs opacity-75">
								{props.file.channel_journey.join(" → ")}
							</span>
						)}
					{props.isLive && <LivePill />}
				</span>
			}
		/>
	);
}
