import type { Dirs, IndividualFile, months } from "../../../Constants";
import { LiveDot } from "./LiveDot";
import { StyledTreeItem } from "./StyledTreeItem";
import { TreeViewMonths } from "./TreeViewMonths";

function anyLive(files: IndividualFile[], liveSet: Set<string>): boolean {
	return files.some((f) => liveSet.has(f.file.slice(0, -4)));
}

export function TreeViewYears(props: {
	el: Dirs;
	index: number;
	liveSet: Set<string>;
}) {
	const months_obj = Object.keys(props.el.months);
	const safe_months = months_obj.map(Number).sort((a, b) => b - a);

	const allFiles: IndividualFile[] = [];
	for (const m of safe_months) {
		const f = props.el.months[m as months];
		if (f) allFiles.push(...f);
	}
	const hasLive = anyLive(allFiles, props.liveSet);

	const result = safe_months.map((month_name, index) => {
		const month = month_name as months;
		const files = props.el.months[month] ?? [];
		return (
			<TreeViewMonths
				files={files}
				index={index}
				month_name={month}
				year={props.el.year}
				liveSet={props.liveSet}
				key={month}
			/>
		);
	});

	return (
		<StyledTreeItem
			className="bg-green-500 overflow-hidden"
			label={
				<span className="inline-flex items-center">
					{props.el.year}
					{hasLive && <LiveDot />}
				</span>
			}
			itemId={`${props.el.year}`}
		>
			<div>{result}</div>
		</StyledTreeItem>
	);
}
