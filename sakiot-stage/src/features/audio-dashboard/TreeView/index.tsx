import * as React from "react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { apiSlice, useGetCurrentGuildDirsQuery } from "../../../app/apiSlice";
import { useAsRole } from "../../../app/useAsRole";
import { type Dirs, getMonthName } from "../../../Constants";
import { SearchInput, Tree } from "../../../shared/ui";
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

export default function RecordingTree(
	props: { onRecordingSelect?: () => void } = {},
) {
	const [expandedItems, setExpandedItems] = useState<string[]>([]);
	const [searchQuery, setSearchQuery] = useState("");
	const params = useParams();
	const location = useLocation();
	const navigate = useNavigate();
	const { asRoleArg } = useAsRole();
	const {
		currentData: channelsData,
		isError,
		error: dirsError,
	} = useGetCurrentGuildDirsQuery(
		{ guild_id: params.guild_id ?? "", ...asRoleArg },
		{
			skip: !params.guild_id,
		},
	);
	const { data: selectedSession } =
		apiSlice.endpoints.getSessionManifest.useQueryState(
			params.session_id ?? "",
			{ skip: !params.session_id },
		);
	const selectedSessionFinalized = selectedSession?.state === "finalized";
	const liveArgs = { guild_id: params.guild_id ?? "", ...asRoleArg };
	const { currentData: liveStems } =
		apiSlice.endpoints.getLiveStems.useQueryState(liveArgs, {
			skip: !params.guild_id,
		});
	apiSlice.endpoints.getLiveStems.useQuerySubscription(liveArgs, {
		skip: !params.guild_id,
		// The route loader takes an initial snapshot. Keep checking only while
		// there are live badges to update; an idle guild needs no polling.
		pollingInterval:
			!selectedSessionFinalized && liveStems?.length ? 10_000 : 0,
	});
	const liveSet = useMemo(() => new Set(liveStems ?? []), [liveStems]);

	const data = useMemo(
		() => (channelsData ? transform_to_months(channelsData) : null),
		[channelsData],
	);

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
	if (isError) {
		const status =
			typeof dirsError === "object" && dirsError && "status" in dirsError
				? (dirsError as { status: number | string }).status
				: null;
		return (
			<div className="w-full rounded-lg bg-surface p-3 text-sm text-red-300">
				{status === 403 && asRoleArg
					? "Role preview access denied. Viewing account must be a guild manager."
					: "Failed to load recordings tree."}
			</div>
		);
	}

	if (!data)
		return <div className="w-full p-2 text-sm text-muted">Loading Tree…</div>;

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
	const toggleExpanded = (itemId: string) => {
		setExpandedItems((prev) =>
			prev.includes(itemId)
				? prev.filter((item) => item !== itemId)
				: [...prev, itemId],
		);
	};
	// React Aria only expands a row from its chevron button. Year/month/day rows
	// carry no route, so pressing anywhere on them toggles the branch instead,
	// while leaves keep navigating. Exactly one of onSelectionChange/onAction
	// fires per press (mouse and Space select, Enter acts), so both dispatch here.
	const handleTreePress = (itemId: string) => {
		if (itemRoutes.has(itemId)) {
			selectRecording(itemId);
			return;
		}
		toggleExpanded(itemId);
	};

	return (
		<div className="w-full rounded-lg bg-surface p-2">
			<SearchInput
				id="audio-tree-search"
				label="Search recordings"
				placeholder="Search..."
				value={searchQuery}
				onChange={setSearchQuery}
			/>
			{visibleData.length > 0 ? (
				<Tree
					aria-label="Recordings"
					selectionMode="single"
					expandedKeys={expandedItems}
					selectedKeys={
						routeState.selectedItemId ? [routeState.selectedItemId] : []
					}
					onExpandedChange={(keys) => setExpandedItems([...keys].map(String))}
					onSelectionChange={(keys) => {
						if (keys !== "all") {
							const key = [...keys][0];
							if (key != null) handleTreePress(String(key));
						}
					}}
					onAction={(key) => handleTreePress(String(key))}
					className="mt-3 w-full space-y-1"
				>
					{years}
				</Tree>
			) : (
				<p className="px-2 py-4 text-sm text-muted">No recordings found.</p>
			)}
		</div>
	);
}
