--
-- PostgreSQL database dump
--

-- Dumped from database version 18.3 (Debian 18.3-1.pgdg12+1)
-- Dumped by pg_dump version 18.3 (Debian 18.3-1.pgdg12+1)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;


--
-- Name: user_or_role; Type: TYPE; Schema: public; Owner: -
--

CREATE TYPE public.user_or_role AS ENUM (
    'user',
    'role'
);


--
-- Name: get_channel_overriders_for_user_id(bigint); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.get_channel_overriders_for_user_id(p_guild_id bigint) RETURNS TABLE(allow bigint, deny bigint, channel_id bigint, name character varying)
    LANGUAGE plpgsql
    AS $$
BEGIN
return query SELECT channel_permissions.allow, channel_permissions.deny,  channels.channel_id, channels.name 
	FROM channels 
	LEFT JOIN channel_permissions 
	ON channels.channel_id=channel_permissions.channel_id OR channel_permissions.channel_id IS NULL
	WHERE channels.type = 2
	AND channels.guild_id=p_guild_id
	AND (channel_permissions.target_id=p_guild_id OR channel_permissions.target_id IS NULL);
	END
$$;


--
-- Name: get_roles_overwrites_for_channels_from_user(bigint, bigint); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.get_roles_overwrites_for_channels_from_user(p_target_id bigint, p_guild_id bigint) RETURNS TABLE(allow bigint, deny bigint, channel_id bigint, role_id bigint)
    LANGUAGE plpgsql
    AS $$
BEGIN
    RETURN QUERY
    SELECT
        channel_permissions.allow,
        channel_permissions.deny,
        channel_permissions.channel_id,
        channel_permissions.target_id
    FROM channel_permissions
    INNER JOIN user_roles ON channel_permissions.target_id = user_roles.role_id
    INNER JOIN channels ON channel_permissions.channel_id = channels.channel_id
    WHERE user_roles.user_id = p_target_id
        AND channel_permissions.kind = 'role'
        AND channels."type" = 2
        AND channels.guild_id = p_guild_id;
END
$$;


--
-- Name: get_user_channel_overriders_for_user_id(bigint, bigint); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.get_user_channel_overriders_for_user_id(p_target_id bigint, p_guild_id bigint) RETURNS TABLE(allow bigint, deny bigint, channel_id bigint)
    LANGUAGE plpgsql
    AS $$
BEGIN
    RETURN QUERY
    SELECT
        channel_permissions.allow,
        channel_permissions.deny,
        channel_permissions.channel_id
    FROM channel_permissions
    INNER JOIN channels ON channel_permissions.channel_id = channels.channel_id
    WHERE channels.type = 2
        AND channels.guild_id = p_guild_id
        AND channel_permissions.kind = 'member'
        AND channel_permissions.target_id = p_target_id;
END
$$;


SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: audio_files; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.audio_files (
    file_name character varying NOT NULL,
    guild_id bigint NOT NULL,
    channel_id bigint NOT NULL,
    user_id bigint NOT NULL,
    year integer NOT NULL,
    month integer NOT NULL,
    start_ts bigint,
    end_ts bigint,
    state_enter smallint DEFAULT 0 NOT NULL,
    silence boolean DEFAULT false NOT NULL,
    state_leave smallint,
    id bigint NOT NULL,
    reaped boolean DEFAULT false NOT NULL,
    waveform_end_ts bigint,
    recording_owner_instance_id text,
    recording_heartbeat_at timestamp with time zone
);


--
-- Name: COLUMN audio_files.waveform_end_ts; Type: COMMENT; Schema: public; Owner: -
--

COMMENT ON COLUMN public.audio_files.waveform_end_ts IS 'Final end_ts used to generate cached waveform .dat; NULL means no finalized waveform cache.';


--
-- Name: audio_files_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.audio_files_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: audio_files_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.audio_files_id_seq OWNED BY public.audio_files.id;


--
-- Name: audio_files_state; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.audio_files_state (
    state smallint NOT NULL,
    meaning character varying
);


--
-- Name: bot_instances; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.bot_instances (
    instance_id text NOT NULL,
    role text NOT NULL,
    state text NOT NULL,
    heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT bot_instances_role_check CHECK ((role = ANY (ARRAY['active'::text, 'drain'::text]))),
    CONSTRAINT bot_instances_state_check CHECK ((state = ANY (ARRAY['active'::text, 'draining'::text, 'stopped'::text])))
);


--
-- Name: bot_reaper_state; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.bot_reaper_state (
    id smallint NOT NULL,
    last_reap_ts bigint DEFAULT 0 NOT NULL,
    CONSTRAINT bot_reaper_state_id_check CHECK ((id = 1))
);


--
-- Name: channel_permissions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.channel_permissions (
    channel_id bigint NOT NULL,
    target_id bigint NOT NULL,
    kind character varying(4) NOT NULL,
    allow bigint NOT NULL,
    deny bigint NOT NULL,
    CONSTRAINT kind CHECK (((kind)::text = ANY (ARRAY[('role'::character varying)::text, ('user'::character varying)::text])))
);


--
-- Name: COLUMN channel_permissions.target_id; Type: COMMENT; Schema: public; Owner: -
--

COMMENT ON COLUMN public.channel_permissions.target_id IS 'General purpose id. Can be of kind User or Role';


--
-- Name: channel_type; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.channel_type (
    id integer NOT NULL,
    type character varying NOT NULL
);


--
-- Name: channel_type_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.channel_type_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: channel_type_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.channel_type_id_seq OWNED BY public.channel_type.id;


--
-- Name: channels; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.channels (
    channel_id bigint NOT NULL,
    guild_id bigint NOT NULL,
    type integer NOT NULL,
    name character varying
);


--
-- Name: channels_channel_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.channels_channel_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: channels_channel_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.channels_channel_id_seq OWNED BY public.channels.channel_id;


--
-- Name: clips; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.clips (
    length real,
    size bigint,
    channel_id bigint,
    guild_id bigint,
    user_id bigint,
    original_file_name character varying(255),
    saved_file_name character varying(255),
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP,
    name character varying(255),
    clip_id character varying(255) NOT NULL,
    start_time real NOT NULL,
    deleted_at timestamp with time zone
);


--
-- Name: discord_auth_user; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.discord_auth_user (
    id bigint NOT NULL,
    username character varying(255) NOT NULL,
    discriminator character varying(255) NOT NULL,
    avatar character varying(255) NOT NULL,
    bot boolean,
    system boolean,
    mfa_enabled boolean,
    banner character varying(255),
    accent_color integer,
    locale character varying(255),
    verified boolean,
    email character varying(255),
    flags integer,
    premium_type integer,
    public_flags integer,
    token_version integer DEFAULT 0 NOT NULL
);


--
-- Name: guild_channels; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.guild_channels (
    guild_id bigint NOT NULL,
    channel_id bigint NOT NULL
);


--
-- Name: guild_jam_cooldowns; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.guild_jam_cooldowns (
    guild_id bigint NOT NULL,
    cooldown_seconds integer DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: guilds; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.guilds (
    id bigint NOT NULL,
    owner_id bigint NOT NULL
);


--
-- Name: guilds_present; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.guilds_present (
    guild_id bigint NOT NULL
);


--
-- Name: jam_invocations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.jam_invocations (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    guild_id bigint NOT NULL,
    clip_id text NOT NULL,
    played_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: jam_invocations_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.jam_invocations_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: jam_invocations_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.jam_invocations_id_seq OWNED BY public.jam_invocations.id;


--
-- Name: kpi_test; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.kpi_test (
    id bigint NOT NULL,
    "Department" character varying DEFAULT 'UNKOWN'::character varying NOT NULL,
    "Type" character varying DEFAULT 'UNKOWN'::character varying NOT NULL,
    "Reported" character varying DEFAULT 'UNKOWN'::character varying NOT NULL,
    "Day" character varying DEFAULT 'UNKOWN'::character varying NOT NULL,
    "Date" date DEFAULT CURRENT_DATE NOT NULL,
    "Reason" character varying DEFAULT 'UNKOWN'::character varying NOT NULL
);


--
-- Name: kpi_test_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.kpi_test_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: kpi_test_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.kpi_test_id_seq OWNED BY public.kpi_test.id;


--
-- Name: roles; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.roles (
    guild_id bigint NOT NULL,
    role_id bigint NOT NULL,
    permission bigint NOT NULL,
    name character varying(50) NOT NULL
);


--
-- Name: stamps; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.stamps (
    id bigint NOT NULL,
    guild_id bigint NOT NULL,
    channel_id bigint NOT NULL,
    target_user_id bigint NOT NULL,
    stamper_user_id bigint NOT NULL,
    stamp_ts bigint NOT NULL,
    offset_ms integer DEFAULT 0 NOT NULL,
    audio_file_id bigint,
    note text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: stamps_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.stamps_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: stamps_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.stamps_id_seq OWNED BY public.stamps.id;


--
-- Name: user_guilds; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_guilds (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    name character varying(255) NOT NULL,
    icon character varying(255),
    owner boolean NOT NULL,
    permissions bigint NOT NULL,
    features text[] NOT NULL
);


--
-- Name: user_jam_cooldown_overrides; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_jam_cooldown_overrides (
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    cooldown_seconds integer NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_name_event_types; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_name_event_types (
    id integer NOT NULL,
    name character varying(50) NOT NULL
);


--
-- Name: user_name_event_types_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.user_name_event_types_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: user_name_event_types_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.user_name_event_types_id_seq OWNED BY public.user_name_event_types.id;


--
-- Name: user_name_history; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_name_history (
    id bigint NOT NULL,
    user_id bigint NOT NULL,
    guild_id bigint,
    kind_id integer NOT NULL,
    value text,
    observed_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_name_history_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.user_name_history_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: user_name_history_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.user_name_history_id_seq OWNED BY public.user_name_history.id;


--
-- Name: user_names; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_names (
    user_id bigint NOT NULL,
    username text NOT NULL,
    global_name text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_nicknames; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_nicknames (
    user_id bigint NOT NULL,
    guild_id bigint NOT NULL,
    nickname text,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_roles; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_roles (
    user_id bigint NOT NULL,
    role_id bigint NOT NULL
);


--
-- Name: users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.users (
    user_id bigint NOT NULL
);


--
-- Name: users_user_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.users_user_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: users_user_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.users_user_id_seq OWNED BY public.users.user_id;


--
-- Name: voice_event_types; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_event_types (
    id integer NOT NULL,
    name character varying(50) NOT NULL
);


--
-- Name: voice_event_types_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.voice_event_types_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: voice_event_types_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.voice_event_types_id_seq OWNED BY public.voice_event_types.id;


--
-- Name: voice_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_events (
    id bigint NOT NULL,
    guild_id bigint NOT NULL,
    channel_id bigint,
    user_id bigint NOT NULL,
    event_type_id integer NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: voice_events_audit; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_events_audit (
    id integer NOT NULL,
    "timestamp" timestamp with time zone DEFAULT now() NOT NULL,
    guild_id bigint NOT NULL,
    user_id bigint NOT NULL,
    ssrc bigint NOT NULL,
    event_type_id integer,
    details text
);


--
-- Name: voice_events_audit_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.voice_events_audit_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: voice_events_audit_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.voice_events_audit_id_seq OWNED BY public.voice_events_audit.id;


--
-- Name: voice_events_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.voice_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: voice_events_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.voice_events_id_seq OWNED BY public.voice_events.id;


--
-- Name: voice_session_leases; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_session_leases (
    guild_id bigint NOT NULL,
    channel_id bigint NOT NULL,
    owner_instance_id text NOT NULL,
    state text NOT NULL,
    heartbeat_at timestamp with time zone DEFAULT now() NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT voice_session_leases_state_check CHECK ((state = ANY (ARRAY['active'::text, 'draining'::text])))
);


--
-- Name: voice_state_event_types; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_state_event_types (
    id integer NOT NULL,
    name character varying(50) NOT NULL
);


--
-- Name: voice_state_event_types_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.voice_state_event_types_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: voice_state_event_types_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.voice_state_event_types_id_seq OWNED BY public.voice_state_event_types.id;


--
-- Name: voice_state_events; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.voice_state_events (
    id bigint NOT NULL,
    guild_id bigint NOT NULL,
    channel_id bigint,
    user_id bigint NOT NULL,
    event_type_id integer NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: voice_state_events_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.voice_state_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: voice_state_events_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.voice_state_events_id_seq OWNED BY public.voice_state_events.id;


--
-- Name: audio_files id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files ALTER COLUMN id SET DEFAULT nextval('public.audio_files_id_seq'::regclass);


--
-- Name: channel_type id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channel_type ALTER COLUMN id SET DEFAULT nextval('public.channel_type_id_seq'::regclass);


--
-- Name: jam_invocations id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jam_invocations ALTER COLUMN id SET DEFAULT nextval('public.jam_invocations_id_seq'::regclass);


--
-- Name: kpi_test id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.kpi_test ALTER COLUMN id SET DEFAULT nextval('public.kpi_test_id_seq'::regclass);


--
-- Name: stamps id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.stamps ALTER COLUMN id SET DEFAULT nextval('public.stamps_id_seq'::regclass);


--
-- Name: user_name_event_types id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_event_types ALTER COLUMN id SET DEFAULT nextval('public.user_name_event_types_id_seq'::regclass);


--
-- Name: user_name_history id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_history ALTER COLUMN id SET DEFAULT nextval('public.user_name_history_id_seq'::regclass);


--
-- Name: voice_event_types id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_event_types ALTER COLUMN id SET DEFAULT nextval('public.voice_event_types_id_seq'::regclass);


--
-- Name: voice_events id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events ALTER COLUMN id SET DEFAULT nextval('public.voice_events_id_seq'::regclass);


--
-- Name: voice_events_audit id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events_audit ALTER COLUMN id SET DEFAULT nextval('public.voice_events_audit_id_seq'::regclass);


--
-- Name: voice_state_event_types id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_event_types ALTER COLUMN id SET DEFAULT nextval('public.voice_state_event_types_id_seq'::regclass);


--
-- Name: voice_state_events id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_events ALTER COLUMN id SET DEFAULT nextval('public.voice_state_events_id_seq'::regclass);


--
-- Name: audio_files audio_files_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files
    ADD CONSTRAINT audio_files_id_key UNIQUE (id);


--
-- Name: audio_files audio_files_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files
    ADD CONSTRAINT audio_files_pkey PRIMARY KEY (file_name);


--
-- Name: audio_files_state audio_files_state_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files_state
    ADD CONSTRAINT audio_files_state_pkey PRIMARY KEY (state);


--
-- Name: bot_instances bot_instances_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.bot_instances
    ADD CONSTRAINT bot_instances_pkey PRIMARY KEY (instance_id);


--
-- Name: bot_reaper_state bot_reaper_state_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.bot_reaper_state
    ADD CONSTRAINT bot_reaper_state_pkey PRIMARY KEY (id);


--
-- Name: channel_permissions chanel_target; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channel_permissions
    ADD CONSTRAINT chanel_target UNIQUE (channel_id, target_id);


--
-- Name: channel_type channel_type_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channel_type
    ADD CONSTRAINT channel_type_pkey PRIMARY KEY (id);


--
-- Name: channels channels_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channels
    ADD CONSTRAINT channels_pkey PRIMARY KEY (channel_id);


--
-- Name: clips clips_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.clips
    ADD CONSTRAINT clips_pkey PRIMARY KEY (clip_id);


--
-- Name: discord_auth_user discord_auth_user_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.discord_auth_user
    ADD CONSTRAINT discord_auth_user_pkey PRIMARY KEY (id);


--
-- Name: guild_jam_cooldowns guild_jam_cooldowns_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.guild_jam_cooldowns
    ADD CONSTRAINT guild_jam_cooldowns_pkey PRIMARY KEY (guild_id);


--
-- Name: guilds guilds_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.guilds
    ADD CONSTRAINT guilds_pkey PRIMARY KEY (id);


--
-- Name: guilds_present guilds_present_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.guilds_present
    ADD CONSTRAINT guilds_present_pkey PRIMARY KEY (guild_id);


--
-- Name: jam_invocations jam_invocations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jam_invocations
    ADD CONSTRAINT jam_invocations_pkey PRIMARY KEY (id);


--
-- Name: kpi_test kpi_test_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.kpi_test
    ADD CONSTRAINT kpi_test_pkey PRIMARY KEY (id);


--
-- Name: roles roles_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_pkey PRIMARY KEY (role_id);


--
-- Name: stamps stamps_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.stamps
    ADD CONSTRAINT stamps_pkey PRIMARY KEY (id);


--
-- Name: user_guilds user_and_guild_id; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_guilds
    ADD CONSTRAINT user_and_guild_id UNIQUE (id, user_id);


--
-- Name: user_jam_cooldown_overrides user_jam_cooldown_overrides_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_jam_cooldown_overrides
    ADD CONSTRAINT user_jam_cooldown_overrides_pkey PRIMARY KEY (guild_id, user_id);


--
-- Name: user_name_event_types user_name_event_types_name_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_event_types
    ADD CONSTRAINT user_name_event_types_name_key UNIQUE (name);


--
-- Name: user_name_event_types user_name_event_types_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_event_types
    ADD CONSTRAINT user_name_event_types_pkey PRIMARY KEY (id);


--
-- Name: user_name_history user_name_history_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_history
    ADD CONSTRAINT user_name_history_pkey PRIMARY KEY (id);


--
-- Name: user_names user_names_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_names
    ADD CONSTRAINT user_names_pkey PRIMARY KEY (user_id);


--
-- Name: user_nicknames user_nicknames_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_nicknames
    ADD CONSTRAINT user_nicknames_pkey PRIMARY KEY (user_id, guild_id);


--
-- Name: user_roles user_roles_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT user_roles_pkey PRIMARY KEY (user_id, role_id);


--
-- Name: users users_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (user_id);


--
-- Name: voice_event_types voice_event_types_name_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_event_types
    ADD CONSTRAINT voice_event_types_name_key UNIQUE (name);


--
-- Name: voice_event_types voice_event_types_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_event_types
    ADD CONSTRAINT voice_event_types_pkey PRIMARY KEY (id);


--
-- Name: voice_events_audit voice_events_audit_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events_audit
    ADD CONSTRAINT voice_events_audit_pkey PRIMARY KEY (id);


--
-- Name: voice_events voice_events_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events
    ADD CONSTRAINT voice_events_pkey PRIMARY KEY (id);


--
-- Name: voice_session_leases voice_session_leases_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_session_leases
    ADD CONSTRAINT voice_session_leases_pkey PRIMARY KEY (guild_id);


--
-- Name: voice_state_event_types voice_state_event_types_name_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_event_types
    ADD CONSTRAINT voice_state_event_types_name_key UNIQUE (name);


--
-- Name: voice_state_event_types voice_state_event_types_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_event_types
    ADD CONSTRAINT voice_state_event_types_pkey PRIMARY KEY (id);


--
-- Name: voice_state_events voice_state_events_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_events
    ADD CONSTRAINT voice_state_events_pkey PRIMARY KEY (id);


--
-- Name: audio_files_guild_id_index; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audio_files_guild_id_index ON public.audio_files USING btree (guild_id);


--
-- Name: audio_files_reaped_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audio_files_reaped_idx ON public.audio_files USING btree (reaped) WHERE (reaped = true);


--
-- Name: audio_files_recording_heartbeat_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audio_files_recording_heartbeat_idx ON public.audio_files USING btree (recording_heartbeat_at) WHERE (recording_heartbeat_at IS NOT NULL);


--
-- Name: audio_files_recording_owner_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX audio_files_recording_owner_idx ON public.audio_files USING btree (recording_owner_instance_id) WHERE (recording_owner_instance_id IS NOT NULL);


--
-- Name: clips_deleted_at_null_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX clips_deleted_at_null_idx ON public.clips USING btree (guild_id) WHERE (deleted_at IS NULL);


--
-- Name: jam_invocations_guild_played_at_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX jam_invocations_guild_played_at_idx ON public.jam_invocations USING btree (guild_id, played_at DESC);


--
-- Name: jam_invocations_played_at_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX jam_invocations_played_at_idx ON public.jam_invocations USING btree (played_at DESC);


--
-- Name: stamps_by_file; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX stamps_by_file ON public.stamps USING btree (audio_file_id);


--
-- Name: stamps_lookup_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX stamps_lookup_idx ON public.stamps USING btree (guild_id, channel_id, target_user_id, stamp_ts);


--
-- Name: user_name_history_lookup; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX user_name_history_lookup ON public.user_name_history USING btree (user_id, observed_at DESC);


--
-- Name: voice_events_channel_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_events_channel_time ON public.voice_events USING btree (guild_id, channel_id, occurred_at);


--
-- Name: voice_events_user_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_events_user_time ON public.voice_events USING btree (user_id, occurred_at);


--
-- Name: voice_session_leases_heartbeat_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_session_leases_heartbeat_idx ON public.voice_session_leases USING btree (heartbeat_at);


--
-- Name: voice_session_leases_owner_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_session_leases_owner_idx ON public.voice_session_leases USING btree (owner_instance_id);


--
-- Name: voice_state_events_channel_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_state_events_channel_time ON public.voice_state_events USING btree (guild_id, channel_id, occurred_at);


--
-- Name: voice_state_events_user_time; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX voice_state_events_user_time ON public.voice_state_events USING btree (user_id, occurred_at);


--
-- Name: channel_permissions FK_channel_permissions_channels; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channel_permissions
    ADD CONSTRAINT "FK_channel_permissions_channels" FOREIGN KEY (channel_id) REFERENCES public.channels(channel_id) ON UPDATE CASCADE ON DELETE CASCADE;


--
-- Name: channels FK_channels_channel_type; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channels
    ADD CONSTRAINT "FK_channels_channel_type" FOREIGN KEY (type) REFERENCES public.channel_type(id) ON UPDATE CASCADE ON DELETE CASCADE;


--
-- Name: channels FK_channels_guilds; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.channels
    ADD CONSTRAINT "FK_channels_guilds" FOREIGN KEY (guild_id) REFERENCES public.guilds(id) ON UPDATE CASCADE ON DELETE CASCADE;


--
-- Name: audio_files audio_files_recording_owner_instance_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files
    ADD CONSTRAINT audio_files_recording_owner_instance_id_fkey FOREIGN KEY (recording_owner_instance_id) REFERENCES public.bot_instances(instance_id) ON DELETE SET NULL;


--
-- Name: roles guild_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT guild_id_fk FOREIGN KEY (guild_id) REFERENCES public.guilds(id) ON UPDATE CASCADE ON DELETE CASCADE;


--
-- Name: user_roles role_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT role_id_fk FOREIGN KEY (role_id) REFERENCES public.roles(role_id) ON UPDATE CASCADE ON DELETE CASCADE;


--
-- Name: stamps stamps_audio_file_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.stamps
    ADD CONSTRAINT stamps_audio_file_id_fkey FOREIGN KEY (audio_file_id) REFERENCES public.audio_files(id) ON DELETE SET NULL;


--
-- Name: audio_files state; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.audio_files
    ADD CONSTRAINT state FOREIGN KEY (state_enter) REFERENCES public.audio_files_state(state) NOT VALID;


--
-- Name: user_name_history user_name_history_kind_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_name_history
    ADD CONSTRAINT user_name_history_kind_id_fkey FOREIGN KEY (kind_id) REFERENCES public.user_name_event_types(id);


--
-- Name: voice_events_audit voice_events_audit_event_type_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events_audit
    ADD CONSTRAINT voice_events_audit_event_type_id_fkey FOREIGN KEY (event_type_id) REFERENCES public.voice_event_types(id);


--
-- Name: voice_events voice_events_event_type_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_events
    ADD CONSTRAINT voice_events_event_type_id_fkey FOREIGN KEY (event_type_id) REFERENCES public.voice_event_types(id);


--
-- Name: voice_session_leases voice_session_leases_owner_instance_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_session_leases
    ADD CONSTRAINT voice_session_leases_owner_instance_id_fkey FOREIGN KEY (owner_instance_id) REFERENCES public.bot_instances(instance_id) ON DELETE CASCADE;


--
-- Name: voice_state_events voice_state_events_event_type_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.voice_state_events
    ADD CONSTRAINT voice_state_events_event_type_id_fkey FOREIGN KEY (event_type_id) REFERENCES public.voice_state_event_types(id);


--
-- PostgreSQL database dump complete
--

