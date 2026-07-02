import { SimpleTreeView } from "@mui/x-tree-view/SimpleTreeView";
import * as React from "react";
import { useMemo, useState } from "react";
import { useParams } from "react-router-dom";
import {
	useGetCurrentGuildDirsQuery,
	useGetLiveStemsQuery,
} from "../../../app/apiSlice";
import type { Dirs, months, UserGuilds } from "../../../Constants";
import { transform_to_months } from "../data";
import { TreeViewYears } from "./TreeViewYears";

// Expansion path to the currently open file, or null if none is selected.
function getFileExpansion(params: {
	year?: string;
	month?: string;
	file_name?: string;
}): string[] | null {
	const year = Number(params.year);
	const month = Number(params.month);
	const fileTimestamp = Number(params.file_name?.slice(0, 13));

	if (
		params.file_name &&
		Number.isFinite(year) &&
		Number.isFinite(month) &&
		Number.isFinite(fileTimestamp)
	) {
		const day = new Date(fileTimestamp).getDate();
		return [`${year}`, `${year}-${month}`, `${year}-${month}-${day}`];
	}

	return null;
}

// Expansion path to the topmost (newest) day actually present in the tree.
// Mirrors the render order: years desc, months desc, days desc.
function getTopmostExpansion(data: Dirs[]): string[] {
	const year = data[0];
	if (!year) return [];

	const topMonth = Object.keys(year.months)
		.map(Number)
		.sort((a, b) => b - a)[0];
	if (topMonth === undefined) return [`${year.year}`];

	const files = year.months[topMonth as months] ?? [];
	let topDay: number | undefined;
	for (const f of files) {
		const day = new Date(parseInt(f.file.slice(0, 13), 10)).getDate();
		if (topDay === undefined || day > topDay) topDay = day;
	}

	const ids = [`${year.year}`, `${year.year}-${topMonth}`];
	if (topDay !== undefined) ids.push(`${year.year}-${topMonth}-${topDay}`);
	return ids;
}

export default function CustomizedTreeView(_props: {
	guildSelected: UserGuilds | null;
}) {
	const [data, setData] = useState<Dirs[] | null>(null);
	const [expandedItems, setExpandedItems] = useState<string[]>([]);
	const params = useParams();
	const { data: channelsData, isSuccess } = useGetCurrentGuildDirsQuery(
		params.guild_id ?? "",
		{
			skip: !params.guild_id,
			refetchOnMountOrArgChange: true,
		},
	);
	const { data: liveStems } = useGetLiveStemsQuery(params.guild_id ?? "", {
		skip: !params.guild_id,
		pollingInterval: 10_000,
	});
	const liveSet = useMemo(() => new Set(liveStems ?? []), [liveStems]);

	React.useEffect(() => {
		if (isSuccess && channelsData) {
			const res = transform_to_months(channelsData);
			setData(res);
		}
	}, [channelsData, isSuccess]);

	// Auto-expand the path to the selected file, or — when none is selected —
	// the topmost day present in the tree. Runs once whenever that target
	// changes (navigation / data load), then expansion is fully user-controlled.
	const requiredItems = useMemo(() => {
		const fileExpansion = getFileExpansion(params);
		if (fileExpansion) return fileExpansion;
		return data ? getTopmostExpansion(data) : [];
	}, [params, data]);
	const requiredKey = requiredItems.join(",");
	React.useEffect(() => {
		if (!requiredKey) return;
		setExpandedItems((prev) =>
			Array.from(new Set([...prev, ...requiredKey.split(",")])),
		);
	}, [requiredKey]);

	if (!data) return <div className="w-full p-3">Loading Tree</div>;

	const years = data.map((el, index) => (
		<TreeViewYears el={el} index={index} liveSet={liveSet} key={el.year} />
	));

	return (
		<SimpleTreeView
			aria-label="customized"
			expandedItems={expandedItems}
			onExpandedItemsChange={(_event, itemIds) => {
				setExpandedItems(itemIds);
			}}
			className="w-full p-3"
		>
			{years}
		</SimpleTreeView>
	);
}
