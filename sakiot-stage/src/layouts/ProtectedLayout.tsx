import { useDispatch } from "react-redux";
import { useNavigate } from "react-router-dom";
import { useGetAuthDetailsQuery } from "../app/apiSlice";
import { setGuildSelected } from "../reducers/appSlice";
import { RouteState } from "../routes/RouteState";

/** The dashboard index is a usable server picker, never a placeholder page. */
export function ProtectedLayout() {
	const navigate = useNavigate();
	const dispatch = useDispatch();
	const { data, isLoading } = useGetAuthDetailsQuery();
	if (isLoading || !data) {
		return (
			<p className="p-6" role="status">
				Loading servers…
			</p>
		);
	}
	const guilds = data.guilds ?? [];
	if (guilds.length === 0) {
		return <RouteState kind="empty" />;
	}
	return (
		<main className="mx-auto max-w-5xl px-6 py-10">
			<h1 className="mb-2 text-2xl font-semibold">Choose a server</h1>
			<p className="mb-6 text-muted">
				Open its audio dashboard to browse recordings and clips.
			</p>
			<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
				{guilds.map((guild) => (
					<button
						key={guild.id}
						type="button"
						className="rounded-lg border border-ui-border bg-surface p-5 text-left hover:border-accent focus-visible:outline-2 focus-visible:outline-focus"
						onClick={() => {
							dispatch(setGuildSelected(guild));
							navigate(`/dashboard/${guild.id}/audio`);
						}}
					>
						{guild.name}
					</button>
				))}
			</div>
		</main>
	);
}
