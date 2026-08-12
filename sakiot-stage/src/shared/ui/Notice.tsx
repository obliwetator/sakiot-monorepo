import { AlertCircle, CheckCircle2, Info, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "./cn";

export type NoticeTone = "info" | "success" | "warning" | "error";
export type NoticeAnnouncement = "alert" | "status";

const tones = {
	info: {
		className: "border-sky-400/35 bg-sky-400/10 text-sky-200",
		Icon: Info,
	},
	success: {
		className: "border-emerald-400/35 bg-emerald-400/10 text-emerald-200",
		Icon: CheckCircle2,
	},
	warning: {
		className: "border-amber-400/35 bg-amber-400/10 text-amber-100",
		Icon: TriangleAlert,
	},
	error: {
		className: "border-red-400/35 bg-red-400/10 text-red-200",
		Icon: AlertCircle,
	},
} satisfies Record<NoticeTone, { className: string; Icon: typeof Info }>;

export interface NoticeProps {
	children: ReactNode;
	tone?: NoticeTone;
	announce?: NoticeAnnouncement;
	className?: string;
}

export function Notice({
	children,
	tone = "info",
	announce,
	className,
}: NoticeProps) {
	const { Icon, className: toneClassName } = tones[tone];
	return (
		<div
			role={announce}
			className={cn(
				"flex min-h-9 items-center gap-2 rounded-md border px-3 py-2 text-sm",
				toneClassName,
				className,
			)}
		>
			<Icon aria-hidden="true" className="size-4 shrink-0" />
			<div>{children}</div>
		</div>
	);
}
