import { Check, X } from "lucide-react";
import type { CapabilityDecision } from "../../app/apiSlice";
import { cn } from "../../shared/ui";

export function Held({ decision }: { decision: CapabilityDecision }) {
	return decision.held ? (
		<>
			<Check aria-hidden="true" className="size-4 text-emerald-300" />
			<span className="sr-only">Allowed</span>
		</>
	) : (
		<>
			<X aria-hidden="true" className="size-4 text-rose-300" />
			<span className="sr-only">Denied</span>
		</>
	);
}

type MarkedKind = Exclude<CapabilityDecision["source"]["kind"], "discord">;

const MARKS: Record<MarkedKind, { label: string; className: string }> = {
	owner: { label: "owner", className: "bg-slate-700 text-slate-200" },
	rule: { label: "rule", className: "bg-accent/20 text-accent" },
	unseen: { label: "hidden", className: "bg-accent/20 text-accent" },
	manager: { label: "you lack", className: "bg-slate-700 text-slate-200" },
	exempt: { label: "exempt", className: "bg-slate-700 text-slate-200" },
};

/**
 * A short mark when something other than Discord decided: a rule, the
 * owner's standing, not seeing the channel, the viewer's own access, or a
 * rule that takes nothing from an editor.
 * `description` says it in full, for screen readers.
 */
export function SourceMark({
	decision,
	description,
}: {
	decision: CapabilityDecision;
	description: string;
}) {
	if (decision.source.kind === "discord") return null;
	const mark = MARKS[decision.source.kind];
	return (
		<span
			className={cn(
				"rounded px-1 text-[0.65rem] uppercase tracking-wide",
				mark.className,
			)}
		>
			<span aria-hidden="true">{mark.label}</span>
			<span className="sr-only">{description}</span>
		</span>
	);
}
