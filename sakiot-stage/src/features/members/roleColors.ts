import type { CSSProperties } from "react";
import type { GuildRole } from "../../app/apiSlice";

function toHex(color: number): string {
	return `#${color.toString(16).padStart(6, "0")}`;
}

/** Non-zero role colors as hex strings, primary first. */
export function roleHexColors(
	role: Pick<GuildRole, "color" | "color_secondary" | "color_tertiary">,
): string[] {
	return [role.color, role.color_secondary, role.color_tertiary]
		.filter((c): c is number => typeof c === "number" && c > 0)
		.map(toHex);
}

/** CSS background for a color swatch; gray for roles without a color. */
export function roleSwatchBackground(
	role: Pick<GuildRole, "color" | "color_secondary" | "color_tertiary">,
): string {
	const colors = roleHexColors(role);
	if (colors.length === 0) return "#bdbdbd";
	if (colors.length === 1) return colors[0];
	return `linear-gradient(90deg, ${colors.join(", ")})`;
}

/** Text style that renders the role name in its color (gradient included). */
export function roleTextStyle(
	role: Pick<GuildRole, "color" | "color_secondary" | "color_tertiary">,
): CSSProperties | undefined {
	const colors = roleHexColors(role);
	if (colors.length === 0) return undefined;
	if (colors.length === 1) return { color: colors[0] };
	return {
		backgroundImage: `linear-gradient(90deg, ${colors.join(", ")})`,
		WebkitBackgroundClip: "text",
		WebkitTextFillColor: "transparent",
	};
}
