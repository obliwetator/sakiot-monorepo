import type { IndividualFileArray } from "../../../Constants";
import { TreeItem, TreeItemContent } from "../../../shared/ui";
import { ItemsEl } from "./ItemsEl";
import { LiveDot } from "./LiveDot";

export function TreeViewDays(props: {
	index: number;
	day: number;
	files: IndividualFileArray;
	year: number;
	month_name: number;
	liveSet: Set<string>;
}) {
	const hasLive = props.files.some(
		(f) => f.state === "active" || props.liveSet.has(f.file.slice(0, -4)),
	);
	const itemsEl = props.files.map((el) => (
		<ItemsEl
			file={el}
			month_name={props.month_name}
			year={props.year}
			isLive={el.state === "active" || props.liveSet.has(el.file.slice(0, -4))}
			key={el.file}
		/>
	));

	return (
		<TreeItem
			id={`${props.year}-${props.month_name}-${props.day}`}
			textValue={String(props.day)}
		>
			<TreeItemContent>
				<span className="inline-flex items-center">
					{props.day}
					{hasLive && <LiveDot />}
				</span>
			</TreeItemContent>
			{itemsEl}
		</TreeItem>
	);
}
