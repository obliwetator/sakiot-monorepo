import { SimpleTreeView } from "@mui/x-tree-view/SimpleTreeView";
import * as React from "react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import {
	apiSlice,
	useGetCurrentGuildDirsQuery,
	useGetLiveStemsQuery,
} from "../../../app/apiSlice";
import { useAsRole } from "../../../app/useAsRole";
import type { Dirs } from "../../../Constants";
import { transform_to_months } from "../data";
import { TreeViewYears } from "./TreeViewYears";
import { audioTreeRouteState, recordingTreeRoutes } from "./treeNavigation";

export default function CustomizedTreeView() {
	const [data, setData] = useState<Dirs[] | null>(null);
	const [expandedItems, setExpandedItems] = useState<string[]>([]);
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

	if (!data) return <div className="w-full p-3">Loading Tree</div>;

	const years = data.map((el, index) => (
		<TreeViewYears el={el} index={index} liveSet={liveSet} key={el.year} />
	));

	return (
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
				const targetPath = itemRoutes.get(itemId);
				if (targetPath && targetPath !== location.pathname) {
					navigate(targetPath + location.search);
				}
			}}
			onSelectedItemsChange={(_event, itemId) => {
				if (!itemId) return;
				const targetPath = itemRoutes.get(itemId);
				if (targetPath && targetPath !== location.pathname) {
					navigate(targetPath + location.search);
				}
			}}
			className="w-full p-3"
		>
			{years}
		</SimpleTreeView>
	);
}
