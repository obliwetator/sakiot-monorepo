import { Box, Chip, Typography } from "../../shared/ui";
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
					<Box
						aria-hidden="true"
						sx={{
							position: "fixed",
							top: precisionZone.topPx,
							left: dragFeedback.plotLeftPx,
							width: dragFeedback.plotWidthPx,
							height: Math.max(0, precisionZone.bottomPx - precisionZone.topPx),
							zIndex: 1_290,
							borderTop: "1px solid rgba(125, 211, 252, 0.5)",
							borderBottom: "1px solid rgba(125, 211, 252, 0.5)",
							bgcolor: "rgba(56, 189, 248, 0.035)",
							pointerEvents: "none",
						}}
					>
						<Chip
							size="small"
							label={
								dragFeedback.multiplier >= 100
									? "Ultra ×100"
									: dragFeedback.multiplier >= 10
										? "Fine ×10"
										: "Normal ×1"
							}
							sx={{
								position: "absolute",
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
								right: 8,
								fontVariantNumeric: "tabular-nums",
							}}
						/>
					</Box>
					{precisionBoundaries.map(
						(boundary) =>
							boundary.yPx >= 0 &&
							boundary.yPx <= globalThis.innerHeight && (
								<Box
									key={boundary.label}
									aria-hidden="true"
									sx={{
										position: "fixed",
										top: boundary.yPx - 15,
										left: dragFeedback.plotLeftPx,
										width: dragFeedback.plotWidthPx,
										height: 30,
										zIndex: 1_291,
										backdropFilter: "blur(7px)",
										background:
											"linear-gradient(180deg, rgba(2, 6, 23, 0), rgba(56, 189, 248, 0.18), rgba(2, 6, 23, 0))",
										borderTop: "1px solid rgba(125, 211, 252, 0.18)",
										borderBottom: "1px solid rgba(125, 211, 252, 0.18)",
										pointerEvents: "none",
									}}
								>
									<Typography
										variant="caption"
										sx={{
											position: "absolute",
											right: 8,
											top: 6,
											color: "primary.light",
											textShadow: "0 1px 2px rgba(2, 6, 23, 0.9)",
										}}
									>
										{boundary.label}
									</Typography>
								</Box>
							),
					)}
				</>
			)}
		</>
	);
}
