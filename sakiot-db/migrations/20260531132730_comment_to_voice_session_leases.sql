-- Add migration script here
comment on table public.voice_session_leases is 'This table acts more like a cache rather that data storage. Active voice connections go here so that they can be tracked';
