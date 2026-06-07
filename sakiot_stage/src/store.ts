import { configureStore } from "@reduxjs/toolkit";
import { apiSlice } from "./app/apiSlice";
import appReducer from "./reducers/appSlice";
import hasSilenceReducer from "./reducers/silence";

export const store = configureStore({
	reducer: {
		[apiSlice.reducerPath]: apiSlice.reducer,
		app: appReducer,
		hasSilence: hasSilenceReducer,
	},
	middleware: (getDefaultMiddleware) =>
		getDefaultMiddleware({
			serializableCheck: {
				ignoredActionPaths: [
					"meta.baseQueryMeta.request",
					"meta.baseQueryMeta.response",
				],
			},
		}).concat(apiSlice.middleware),
});

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
