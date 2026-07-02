import { useLocation, useNavigate, useParams } from "react-router-dom";
import {
	type IndividualFile,
	PATH_PREFIX_FOR_LOGGED_USERS,
} from "../../../Constants";
import { LivePill } from "./LiveDot";
import { parseFileName } from "./parseFileName";

export function ItemsEl(props: {
	file: IndividualFile;
	year: number;
	month_name: number;
	isLive?: boolean;
}) {
	const navigate = useNavigate();
	const location = useLocation();
	const params = useParams();

	const fileId = props.file.file.slice(0, -4);
	const targetPath = `${PATH_PREFIX_FOR_LOGGED_USERS}/${params.guild_id}/audio/${props.file.channel_id}/${props.year}/${props.month_name}/${fileId}`;
	const isActive = location.pathname === targetPath;

	const baseColor = "bg-violet-600";
	const hoverColor = "hover:bg-violet-500";
	const activeRing = isActive ? "ring-2 ring-inset ring-white" : "";

	const { time, username: legacyUsername } = parseFileName(props.file.file);
	const username = props.file.display_name ?? legacyUsername;

	return (
		<button
			type="button"
			id={props.file.file}
			className={`${baseColor} ${hoverColor} ${activeRing} w-full text-left px-2 py-1 border-b border-violet-900 last:border-b-0 cursor-pointer select-none text-sm`}
			onClick={(e) => {
				e.preventDefault();
				if (!isActive) navigate(targetPath + location.search);
			}}
			onKeyDown={(e) => {
				if (e.key === "Enter" || e.key === " ") {
					e.preventDefault();
					if (!isActive) navigate(targetPath + location.search);
				}
			}}
			title={props.file.file}
		>
			<span className="font-mono">{time}</span>
			{username && <span className="ml-2">{username}</span>}
			{props.isLive && <LivePill />}
		</button>
	);
}
