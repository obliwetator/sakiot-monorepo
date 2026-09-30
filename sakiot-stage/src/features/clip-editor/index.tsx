import { useParams, useSearchParams } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../../app/apiSlice";
import { ClipEditor } from "./ClipEditor";

export default function ClipEditorPage() {
	const params = useParams<{ guild_id: string }>();
	const [searchParams] = useSearchParams();
	const { data: auth } = useGetAuthDetailsQuery();
	const userId = auth?.user?.user_id;
	const guildId = params.guild_id;
	if (!guildId) return null;
	if (!userId) {
		return (
			<p className="p-6" role="status">
				Loading editor…
			</p>
		);
	}
	const sourceClipId = searchParams.get("source");
	// A draft belongs to one user, guild and source clip. Remounting when any
	// of them changes ends the previous session: its unmount writes its own
	// draft, and none of its timers or callbacks reach the next one.
	return (
		<ClipEditor
			key={JSON.stringify([userId, guildId, sourceClipId])}
			guildId={guildId}
			userId={userId}
			sourceClipId={sourceClipId}
		/>
	);
}
