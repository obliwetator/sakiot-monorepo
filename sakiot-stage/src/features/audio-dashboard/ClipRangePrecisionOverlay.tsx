import { Badge } from "../../shared/ui";
import type { ClipRangeViewportController } from "./useClipRangeViewport";

export function ClipRangePrecisionOverlay({
	controller,
}: {
	controller: ClipRangeViewportController;
}) {
	const { dragFeedback, precisionZone, precisionBoundaries } = controller;
	return (
		<>
			{dragFeedback && precisionZone && dragFeedback.multiplier > 1 && (
				<>
					<div
						aria-hidden="true"
						className="fixed z-1290 border-y border-sky-300/50 bg-info/[3.5%] pointer-events-none"
						style={{
							top: precisionZone.topPx,
							left: dragFeedback.plotLeftPx,
							width: dragFeedback.plotWidthPx,
							height: Math.max(0, precisionZone.bottomPx - precisionZone.topPx),
						}}
					>
						<Badge
							className="absolute right-2 tabular-nums"
							style={{
								top: Math.min(
									Math.max(
										dragFeedback.pointerYPx - precisionZone.topPx - 13,
										4,
									),
									Math.max(
										4,
										precisionZone.bottomPx - precisionZone.topPx - 30,
									),
								),
							}}
							size={"sm"}
						>
							{dragFeedback.multiplier >= 100
								? "Ultra ×100"
								: dragFeedback.multiplier >= 10
									? "Fine ×10"
									: "Normal ×1"}
						</Badge>
					</div>
					{precisionBoundaries.map(
						(boundary) =>
							boundary.yPx >= 0 &&
							boundary.yPx <= globalThis.innerHeight && (
								<div
									key={boundary.label}
									aria-hidden="true"
									className="fixed h-7.5 z-1291 backdrop-blur-[7px] bg-linear-to-b from-transparent via-info/18 to-transparent border-y border-sky-300/18 pointer-events-none"
									style={{
										top: boundary.yPx - 15,
										left: dragFeedback.plotLeftPx,
										width: dragFeedback.plotWidthPx,
									}}
								>
									<span className="text-xs leading-5 absolute right-2 top-1.5 text-focus text-shadow-[0_1px_2px_rgba(2,6,23,0.9)]">
										{boundary.label}
									</span>
								</div>
							),
					)}
				</>
			)}
		</>
	);
}
