import { useSearchParams } from "react-router-dom";

export type AsRoleArg = { as_role: string } | undefined;

export function useAsRole(): {
	asRole: string | undefined;
	asRoleArg: AsRoleArg;
} {
	const [searchParams] = useSearchParams();
	const asRole = searchParams.get("as_role") ?? undefined;
	return { asRole, asRoleArg: asRole ? { as_role: asRole } : undefined };
}
