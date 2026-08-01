import ReactDOM from "react-dom/client";
import { Provider } from "react-redux";
import "@fontsource-variable/inter";
import App from "./App";
import "./index.css";
import { ErrorBoundary } from "./app/ErrorBoundary";
import { store } from "./store";

// const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
	// <React.StrictMode>
	<ErrorBoundary>
		<Provider store={store}>
			<App />
		</Provider>
	</ErrorBoundary>,
	// {/* </React.StrictMode> */}
);
