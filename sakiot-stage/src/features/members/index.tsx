import { Eye as RemoveRedEyeIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import type { GuildRole } from "../../app/apiSlice";
import {
	useGetGuildRolesQuery,
	useGetRoleMembersQuery,
} from "../../app/apiSlice";
import { LoadFailure } from "../../shared/LoadFailure";
import {
	Badge,
	Button,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	TooltipIconButton,
} from "../../shared/ui";
import { RolePreviewDialog } from "./RolePreviewDialog";
import { roleSwatchBackground, roleTextStyle } from "./roleColors";

export function GuildMembers() {
	const { guild_id } = useParams<{ guild_id: string }>();
	const gid = guild_id ?? "";

	const {
		data: roles,
		isLoading: loadingRoles,
		isError: rolesError,
		error: rolesLoadError,
		refetch: refetchRoles,
	} = useGetGuildRolesQuery(gid, { skip: !gid });
	const [selectedRole, setSelectedRole] = useState<GuildRole | null>(null);
	const [previewRole, setPreviewRole] = useState<GuildRole | null>(null);

	useEffect(() => {
		const stillListed = (roles ?? []).some(
			(role) => role.role_id === selectedRole?.role_id,
		);
		if (!stillListed) setSelectedRole(roles?.[0] ?? null);
	}, [roles, selectedRole]);

	const {
		data: members,
		isLoading: loadingMembers,
		isError: membersError,
		error: membersLoadError,
		refetch: refetchMembers,
	} = useGetRoleMembersQuery(
		{ guild_id: gid, role_id: selectedRole?.role_id ?? "" },
		{ skip: !gid || !selectedRole },
	);

	if (!gid) return <div className="p-4">Missing guild id.</div>;

	return (
		<div className="p-4">
			<h5 className="font-semibold tracking-tight text-2xl mb-2">
				Members &amp; roles
			</h5>

			{loadingRoles ? (
				<p className="leading-6">Loading roles…</p>
			) : rolesError ? (
				<LoadFailure
					error={rolesLoadError}
					what="Could not load roles."
					onRetry={() => void refetchRoles()}
				/>
			) : (
				<div className="flex items-stretch flex-col min-[900px]:flex-row gap-4">
					<div className="rounded-md border border-ui-border bg-surface text-fg shadow-none min-[900px]:min-w-70">
						<div className="flex flex-col gap-1">
							{(roles ?? []).map((role) => (
								<div key={role.role_id} className="relative flex items-center">
									<Button
										aria-pressed={selectedRole?.role_id === role.role_id}
										className="w-full justify-start text-left"
										variant="ghost"
										onPress={() => setSelectedRole(role)}
									>
										<span
											aria-hidden="true"
											className="w-3.5 h-3.5 rounded-sm shrink-0 mr-3"
											style={{ background: roleSwatchBackground(role) }}
										/>
										<div>
											{role.name}
											<span className="block text-xs text-muted">
												<Badge size={"sm"}>
													{memberCountLabel(role.member_count)}
												</Badge>
											</span>
										</div>
									</Button>
									<div className="shrink-0 px-2">
										<TooltipIconButton
											label={`View as ${role.name}`}
											tip="View server as this role"
											icon={<RemoveRedEyeIcon size={16} />}
											onPress={() => setPreviewRole(role)}
										/>
									</div>
								</div>
							))}
							{(!roles || roles.length === 0) && (
								<div className="p-4">
									<p className="text-muted text-sm">
										No roles in this guild yet.
									</p>
								</div>
							)}
						</div>
					</div>

					<div className="w-full overflow-x-auto flex-1">
						{selectedRole && (
							<h6 className="text-base p-4">
								<span style={{ ...roleTextStyle(selectedRole) }}>
									{selectedRole.name}
								</span>{" "}
								— {members?.length ?? 0}{" "}
								{members?.length === 1 ? "member" : "members"}
							</h6>
						)}
						{loadingMembers ? (
							<p className="leading-6 p-4">Loading members…</p>
						) : membersError ? (
							<LoadFailure
								className="p-4"
								error={membersLoadError}
								what="Could not load members."
								onRetry={() => void refetchMembers()}
							/>
						) : (
							<Table>
								<TableHeader>
									<TableRow>
										<TableHead>Name</TableHead>
										<TableHead>User ID</TableHead>
									</TableRow>
								</TableHeader>
								<TableBody>
									{(members ?? []).map((member) => (
										<TableRow key={member.user_id}>
											<TableCell>
												{member.name ?? <em>unknown user</em>}
											</TableCell>
											<TableCell>{member.user_id}</TableCell>
										</TableRow>
									))}
									{(!members || members.length === 0) && (
										<TableRow>
											<TableCell colSpan={2}>
												<p className="text-muted text-sm">
													{selectedRole
														? "No members hold this role."
														: "Select a role to see its members."}
												</p>
											</TableCell>
										</TableRow>
									)}
								</TableBody>
							</Table>
						)}
					</div>
				</div>
			)}

			<RolePreviewDialog
				open={previewRole !== null}
				guildId={gid}
				role={previewRole}
				roles={roles ?? []}
				onClose={() => setPreviewRole(null)}
			/>
		</div>
	);
}

/** `null` (only `@everyone`, before the bot has a complete roster): unknown. */
function memberCountLabel(count: number | null | undefined): string {
	if (count === null || count === undefined) return "Member count unknown";
	return `${count} member${count === 1 ? "" : "s"}`;
}
