import { Component, type ErrorInfo, type ReactNode } from "react";

interface ErrorBoundaryState {
	error: Error | null;
}

/**
 * Last-resort catch for render-time throws so a single broken component
 * cannot white-screen the whole SPA. Renders plain DOM with inline styles
 * on purpose: the MUI ThemeProvider lives inside this boundary and may be
 * part of what threw.
 */
export class ErrorBoundary extends Component<
	{ children: ReactNode },
	ErrorBoundaryState
> {
	state: ErrorBoundaryState = { error: null };

	static getDerivedStateFromError(error: Error): ErrorBoundaryState {
		return { error };
	}

	componentDidCatch(error: Error, info: ErrorInfo) {
		console.error("Unhandled render error", error, info.componentStack);
	}

	render() {
		if (this.state.error === null) {
			return this.props.children;
		}
		return (
			<div
				style={{
					minHeight: "100vh",
					display: "flex",
					flexDirection: "column",
					alignItems: "center",
					justifyContent: "center",
					gap: "1rem",
					padding: "2rem",
					textAlign: "center",
					backgroundColor: "#121212",
					color: "#ffffff",
					fontFamily: "Roboto, Helvetica, Arial, sans-serif",
				}}
			>
				<h1 style={{ margin: 0 }}>Something went wrong</h1>
				<p style={{ margin: 0, opacity: 0.7 }}>
					{this.state.error.message || "Unknown render error"}
				</p>
				<button
					type="button"
					onClick={() => window.location.reload()}
					style={{
						padding: "0.5rem 1.5rem",
						borderRadius: "4px",
						border: "1px solid #666",
						backgroundColor: "transparent",
						color: "inherit",
						font: "inherit",
						cursor: "pointer",
					}}
				>
					Reload
				</button>
			</div>
		);
	}
}
