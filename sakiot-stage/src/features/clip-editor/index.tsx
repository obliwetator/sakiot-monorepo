import { useParams } from "react-router-dom";
import { ClipEditor } from "./ClipEditor";

export default function ClipEditorPage() {
	const params = useParams<{ guild_id: string }>();
	if (!params.guild_id) return null;
	return <ClipEditor guildId={params.guild_id} />;
}
