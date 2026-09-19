import { Link } from "react-router-dom";

type RouteStateKind = "empty" | "forbidden" | "not-found";

const copy = {
	empty: {
		title: "No servers available",
		message:
			"Your account has no servers using Sakiot yet. Join a server with the bot, then refresh this page.",
	},
	forbidden: {
		title: "Server access unavailable",
		message:
			"This server is not available to your account. Choose a server you can access.",
	},
	"not-found": {
		title: "Page not found",
		message: "The address may be mistyped or the page may have moved.",
	},
} satisfies Record<RouteStateKind, { title: string; message: string }>;

export function RouteState({ kind }: { kind: RouteStateKind }) {
	const { title, message } = copy[kind];
	return (
		<main className="mx-auto flex min-h-[60vh] max-w-xl flex-col items-start justify-center gap-4 px-6 py-12">
			<h1 className="text-2xl font-semibold">{title}</h1>
			<p className="text-muted">{message}</p>
			{kind === "empty" ? (
				<button
					type="button"
					className="rounded-md border border-ui-border px-4 py-2 hover:bg-surface"
					onClick={() => window.location.reload()}
				>
					Refresh servers
				</button>
			) : (
				<Link
					className="rounded-md border border-ui-border px-4 py-2 hover:bg-surface"
					to="/dashboard"
				>
					Choose a server
				</Link>
			)}
		</main>
	);
}
