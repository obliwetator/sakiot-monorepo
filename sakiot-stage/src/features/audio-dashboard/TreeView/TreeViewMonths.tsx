import { getMonthName, type IndividualFileArray } from "../../../Constants";
import { LiveDot } from "./LiveDot";
import { StyledTreeItem } from "./StyledTreeItem";
import { TreeViewDays } from "./TreeViewDays";
import { recordingFileDay } from "./treeNavigation";

export function TreeViewMonths(props: {
	files: IndividualFileArray | null;
	month_name: number;
	year: number;
	index: number;
	liveSet: Set<string>;
}) {
	const safeFiles = props.files || [];
	const hasLive = safeFiles.some(
		(f) => f.state === "active" || props.liveSet.has(f.file.slice(0, -4)),
	);

	const byDay = new Map<number, IndividualFileArray>();
	for (const el of safeFiles) {
		const day = recordingFileDay(el.file);
		const bucket = byDay.get(day) ?? [];
		bucket.push({ ...el });
		byDay.set(day, bucket);
	}
	for (const bucket of byDay.values()) {
		bucket.sort((a, b) => b.file.localeCompare(a.file));
	}
	const sortedDays = [...byDay.keys()].sort((a, b) => b - a);

	const days = sortedDays.map((day, index) => (
		<TreeViewDays
			index={index}
			day={day}
			files={byDay.get(day) ?? []}
			month_name={props.month_name}
			year={props.year}
			liveSet={props.liveSet}
			key={day}
		/>
	));

	return (
		<StyledTreeItem
			label={
				<span className="inline-flex items-center">
					{getMonthName(props.month_name)}
					{hasLive && <LiveDot />}
				</span>
			}
			itemId={`${props.year}-${props.month_name}`}
		>
			{days}
		</StyledTreeItem>
	);
}
