import { Search } from "lucide-react";
import * as React from "react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import {
	apiSlice,
	useGetCurrentGuildDirsQuery,
	useGetLiveStemsQuery,
} from "../../../app/apiSlice";
import { useAsRole } from "../../../app/useAsRole";
import { type Dirs, getMonthName } from "../../../Constants";
import { SimpleTreeView } from "../../../shared/ui";
import { transform_to_months } from "../data";
import { TreeViewYears } from "./TreeViewYears";
import { audioTreeRouteState, recordingTreeRoutes } from "./treeNavigation";

function filterTree(data: Dirs[], query: string): Dirs[] {
	const needle = query.trim().toLowerCase();
	if (!needle) return data;

	return data.flatMap((year) => {
		const yearMatches = String(year.year).includes(needle);
		const months: Dirs["months"] = {};
		for (const [monthKey, files] of Object.entries(year.months)) {
			const month = Number(monthKey);
			const monthMatches = `${month} ${getMonthName(month)}`
				.toLowerCase()
				.includes(needle);
			const visibleFiles =
				yearMatches || monthMatches
					? (files ?? [])
					: (files ?? []).filter((file) =>
							[
								file.file,
								file.display_name,
								file.user_id,
								file.channel_id,
								...(file.channel_journey ?? []),
							]
								.filter((value): value is string => Boolean(value))
								.some((value) => value.toLowerCase().includes(needle)),
						);
			if (visibleFiles.length > 0) months[month] = visibleFiles;
		}
		return Object.keys(months).length > 0 ? [{ ...year, months }] : [];
	});
}

export default function CustomizedTreeView(
	props: { onRecordingSelect?: () => void } = {},
) {
	const [data, setData] = useState<Dirs[] | null>(null);
	const [expandedItems, setExpandedItems] = useState<string[]>([]);
	const [searchQuery, setSearchQuery] = useState("");
	const params = useParams();
	const location = useLocation();
	const navigate = useNavigate();
	const { asRoleArg } = useAsRole();
	const { data: channelsData, isSuccess } = useGetCurrentGuildDirsQuery(
		{ guild_id: params.guild_id ?? "", ...asRoleArg },
		{
			skip: !params.guild_id,
			refetchOnMountOrArgChange: true,
		},
	);
	const { data: selectedSession } =
		apiSlice.endpoints.getSessionManifest.useQueryState(
			params.session_id ?? "",
			{ skip: !params.session_id },
		);
	const selectedSessionFinalized = selectedSession?.state === "finalized";
	const { data: liveStems } = useGetLiveStemsQuery(
		{ guild_id: params.guild_id ?? "", ...asRoleArg },
		{
			skip: !params.guild_id,
			pollingInterval: selectedSessionFinalized ? 0 : 10_000,
		},
	);
	const liveSet = useMemo(() => new Set(liveStems ?? []), [liveStems]);

	React.useEffect(() => {
		if (isSuccess && channelsData) {
			const res = transform_to_months(channelsData);
			setData(res);
		}
	}, [channelsData, isSuccess]);

	// Resolve the current URL against the loaded tree. This handles both legacy
	// physical-file routes and logical-session routes used by stamps.
	const routeState = useMemo(
		() =>
			data
				? audioTreeRouteState(data, {
						channel_id: params.channel_id,
						file_name: params.file_name,
						month: params.month,
						session_id: params.session_id,
						year: params.year,
					})
				: { expandedItems: [], selectedItemId: null },
		[
			data,
			params.channel_id,
			params.file_name,
			params.month,
			params.session_id,
			params.year,
		],
	);
	const itemRoutes = useMemo(
		() => (data ? recordingTreeRoutes(data, params.guild_id ?? "") : new Map()),
		[data, params.guild_id],
	);
	const requiredItems = routeState.expandedItems;
	const requiredKey = requiredItems.join(",");
	React.useEffect(() => {
		if (!requiredKey) return;
		setExpandedItems((prev) =>
			Array.from(new Set([...prev, ...requiredKey.split(",")])),
		);
	}, [requiredKey]);

	const visibleData = useMemo(
		() => (data ? filterTree(data, searchQuery) : []),
		[data, searchQuery],
	);
	if (!data) return <div className="w-full">Loading Tree</div>;

	const years = visibleData.map((el, index) => (
		<TreeViewYears el={el} index={index} liveSet={liveSet} key={el.year} />
	));
	const selectRecording = (itemId: string) => {
		const targetPath = itemRoutes.get(itemId);
		if (!targetPath) return;
		props.onRecordingSelect?.();
		if (targetPath !== location.pathname) {
			navigate(targetPath + location.search);
		}
	};

	return (
		<div className="w-full rounded-lg bg-surface p-2">
			<label className="relative block" htmlFor="audio-tree-search">
				<Search
					aria-hidden="true"
					className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted"
				/>
				<input
					id="audio-tree-search"
					aria-label="Search recordings"
					value={searchQuery}
					onChange={(event) => setSearchQuery(event.currentTarget.value)}
					placeholder="Search..."
					className="h-9 w-full rounded-md border border-ui-border bg-canvas pl-9 pr-3 text-sm text-fg outline-hidden placeholder:text-muted focus:border-primary focus-visible:outline-2 focus-visible:outline-primary focus-visible:outline-offset-1"
				/>
			</label>
			{visibleData.length > 0 ? (
				<SimpleTreeView
					aria-label="customized"
					expandedItems={expandedItems}
					selectedItems={routeState.selectedItemId}
					onExpandedItemsChange={(_event, itemIds) => {
						setExpandedItems(itemIds);
					}}
					onItemClick={(_event, itemId) => {
						// Selecting an already-selected item does not emit a selection change.
						// Still allow a physical URL mapped to a logical item to open its
						// canonical session route when clicked.
						if (itemId !== routeState.selectedItemId) return;
						selectRecording(itemId);
					}}
					onSelectedItemsChange={(_event, itemId) => {
						if (!itemId) return;
						selectRecording(itemId);
					}}
					className="mt-3 w-full space-y-1"
				>
					{years}
				</SimpleTreeView>
			) : (
				<p className="px-2 py-4 text-sm text-muted">No recordings found.</p>
			)}
		</div>
	);
}
