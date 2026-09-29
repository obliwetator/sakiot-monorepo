-- Makes :owner the owner of a runtime database and of every object in its
-- public schema, so the instance can connect and its migrations can alter
-- anything, and closes the database to every other role.
--
--   sudo -u postgres psql -v ON_ERROR_STOP=1 -v owner=<role> -d <db> -f own-public-schema.sql
--
-- Objects move one by one. REASSIGN OWNED would be shorter but also moves
-- shared objects, including every other database the previous owner holds
-- (production's among them); REASSIGN OWNED BY postgres is refused outright.
-- Indexes follow their table, and sequences linked to a column follow it too.

SELECT set_config('sakiot.owner', :'owner', false) AS owner \gset

DO $$
DECLARE
    new_owner text := current_setting('sakiot.owner');
    object record;
BEGIN
    EXECUTE format('ALTER DATABASE %I OWNER TO %I', current_database(), new_owner);
    EXECUTE format('REVOKE CONNECT, TEMPORARY ON DATABASE %I FROM PUBLIC', current_database());
    EXECUTE format('GRANT USAGE, CREATE ON SCHEMA public TO %I', new_owner);

    FOR object IN
        SELECT tablename FROM pg_tables
         WHERE schemaname = 'public' AND tableowner <> new_owner
    LOOP
        EXECUTE format('ALTER TABLE public.%I OWNER TO %I', object.tablename, new_owner);
    END LOOP;
    FOR object IN
        SELECT sequencename FROM pg_sequences
         WHERE schemaname = 'public' AND sequenceowner <> new_owner
    LOOP
        EXECUTE format('ALTER SEQUENCE public.%I OWNER TO %I', object.sequencename, new_owner);
    END LOOP;
    FOR object IN
        SELECT viewname FROM pg_views
         WHERE schemaname = 'public' AND viewowner <> new_owner
    LOOP
        EXECUTE format('ALTER VIEW public.%I OWNER TO %I', object.viewname, new_owner);
    END LOOP;
    -- Migrations replace functions (CREATE OR REPLACE) and extend enums,
    -- both of which need ownership.
    FOR object IN
        SELECT p.oid::regprocedure AS routine
          FROM pg_proc p
          JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.prokind IN ('f', 'p')
           AND pg_get_userbyid(p.proowner) <> new_owner
           AND NOT EXISTS (
               SELECT 1 FROM pg_depend d
                WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
    LOOP
        EXECUTE format('ALTER ROUTINE %s OWNER TO %I', object.routine, new_owner);
    END LOOP;
    FOR object IN
        SELECT t.typname, t.typtype
          FROM pg_type t
          JOIN pg_namespace n ON n.oid = t.typnamespace
         WHERE n.nspname = 'public'
           AND t.typtype IN ('e', 'd')
           AND pg_get_userbyid(t.typowner) <> new_owner
           AND NOT EXISTS (
               SELECT 1 FROM pg_depend d
                WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e')
    LOOP
        IF object.typtype = 'd' THEN
            EXECUTE format('ALTER DOMAIN public.%I OWNER TO %I', object.typname, new_owner);
        ELSE
            EXECUTE format('ALTER TYPE public.%I OWNER TO %I', object.typname, new_owner);
        END IF;
    END LOOP;
END $$;
