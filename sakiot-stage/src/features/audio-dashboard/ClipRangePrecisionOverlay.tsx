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
						className="fixed [z-index:1290] [border-top:1px_solid_rgba(125,_211,_252,_0.5)] [border-bottom:1px_solid_rgba(125,_211,_252,_0.5)] [background-color:rgba(56,_189,_248,_0.035)] pointer-events-none"
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
									className="fixed h-7.5 [z-index:1291] [backdrop-filter:blur(7px)] [background:linear-gradient(180deg,_rgba(2,_6,_23,_0),_rgba(56,_189,_248,_0.18),_rgba(2,_6,_23,_0))] [border-top:1px_solid_rgba(125,_211,_252,_0.18)] [border-bottom:1px_solid_rgba(125,_211,_252,_0.18)] pointer-events-none"
									style={{
										top: boundary.yPx - 15,
										left: dragFeedback.plotLeftPx,
										width: dragFeedback.plotWidthPx,
									}}
								>
									<span className="text-xs leading-5 absolute right-2 top-1.5 text-focus [text-shadow:0_1px_2px_rgba(2,_6,_23,_0.9)]">
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
