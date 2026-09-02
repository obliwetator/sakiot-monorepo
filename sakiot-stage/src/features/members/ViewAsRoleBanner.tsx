import { Eye as RemoveRedEyeIcon } from "lucide-react";
import { useSearchParams } from "react-router-dom";
import { useGetGuildRolesQuery } from "../../app/apiSlice";
import { Button, Notice } from "../../shared/ui";

export function ViewAsRoleBanner({ guildId }: { guildId: string }) {
	const [searchParams, setSearchParams] = useSearchParams();
	const asRole = searchParams.get("as_role");
	const { data: roles } = useGetGuildRolesQuery(guildId, {
		skip: !guildId || !asRole,
	});

	if (!asRole) return null;
	const roleName = roles?.find((role) => role.role_id === asRole)?.name;

	const exitPreview = () => {
		const next = new URLSearchParams(searchParams);
		next.delete("as_role");
		setSearchParams(next);
	};

	return (
		<div className="mb-4">
			<Notice tone="info" className="flex items-center justify-between gap-3">
				<div className="flex items-center gap-2">
					<RemoveRedEyeIcon
						aria-hidden="true"
						className="size-4 shrink-0 text-sky-400"
					/>
					<span>
						Viewing as <strong>{roleName ?? `role ${asRole}`}</strong> — every
						session stays visible: <em>can't listen</em> marks channels this
						role can see but not join, <em>hidden</em> marks channels it can't
						see at all. Playing or downloading media still uses your own
						permissions.
					</span>
				</div>
				<Button
					variant="ghost"
					size="sm"
					onClick={exitPreview}
					className="shrink-0 text-sky-300 hover:text-sky-100"
				>
					Exit preview
				</Button>
			</Notice>
		</div>
	);
}
