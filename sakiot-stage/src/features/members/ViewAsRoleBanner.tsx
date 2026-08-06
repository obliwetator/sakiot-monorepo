import RemoveRedEyeIcon from "@mui/icons-material/RemoveRedEye";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import { useSearchParams } from "react-router-dom";
import { useGetGuildRolesQuery } from "../../app/apiSlice";

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
		<Box sx={{ mb: 2 }}>
			<Alert
				severity="info"
				variant="outlined"
				icon={<RemoveRedEyeIcon />}
				action={
					<Button color="inherit" size="small" onClick={exitPreview}>
						Exit preview
					</Button>
				}
			>
				Viewing as <strong>{roleName ?? `role ${asRole}`}</strong> — every
				session stays visible: <em>can't listen</em> marks channels this role
				can see but not join, <em>hidden</em> marks channels it can't see at
				all. Playing or downloading media still uses your own permissions.
			</Alert>
		</Box>
	);
}
