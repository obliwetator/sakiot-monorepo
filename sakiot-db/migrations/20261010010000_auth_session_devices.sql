-- What the signed-in devices list shows for each session: the browser and
-- platform named by the User-Agent at sign-in, such as "Chrome" and
-- "Android". Only those names are kept, never the User-Agent string or an IP
-- address. NULL means the sign-in predates this column or was not recognized.
ALTER TABLE public.auth_sessions
    ADD COLUMN browser text,
    ADD COLUMN platform text;
