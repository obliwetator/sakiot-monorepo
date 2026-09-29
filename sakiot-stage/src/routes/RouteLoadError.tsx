import { Link } from "react-router-dom";

export function RouteLoadError({ page = "page" }: { page?: string }) {
	return (
		<main className="p-6 space-y-4" role="alert">
			<h1 className="text-xl font-semibold">Could not load the {page}</h1>
			<p>Reload to try again, or choose another server.</p>
			<button
				type="button"
				className="rounded-md border border-ui-border px-4 py-2"
				onClick={() => window.location.reload()}
			>
				Reload
			</button>
			<Link className="block underline" to="/dashboard">
				Choose a server
			</Link>
		</main>
	);
}
