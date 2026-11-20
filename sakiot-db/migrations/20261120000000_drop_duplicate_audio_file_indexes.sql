-- 20260601000000 made `audio_files.id` the primary key and `file_name`
-- unique, but kept the baseline's `UNIQUE (id)` and added a plain index on
-- `file_name`: two indexes duplicating `audio_files_pkey` and
-- `audio_files_file_name_key`, maintained on every recording insert.
--
-- The foreign keys from `stamps` and `media_objects` were bound to
-- `audio_files_id_key` when they were created, so they are recreated first;
-- with only the primary key left, they bind to it. Their definitions are
-- unchanged.
--
-- `get_channel_overriders_for_user_id` dates from the baseline and nothing
-- calls it.

SET lock_timeout = '5s';

ALTER TABLE public.stamps DROP CONSTRAINT stamps_audio_file_id_fkey;
ALTER TABLE public.media_objects DROP CONSTRAINT media_objects_audio_file_id_fkey;

ALTER TABLE public.audio_files DROP CONSTRAINT audio_files_id_key;
DROP INDEX public.audio_files_file_name_idx;

ALTER TABLE public.stamps
    ADD CONSTRAINT stamps_audio_file_id_fkey
    FOREIGN KEY (audio_file_id) REFERENCES public.audio_files (id) ON DELETE SET NULL;
ALTER TABLE public.media_objects
    ADD CONSTRAINT media_objects_audio_file_id_fkey
    FOREIGN KEY (audio_file_id) REFERENCES public.audio_files (id) ON DELETE RESTRICT;

DROP FUNCTION public.get_channel_overriders_for_user_id(bigint);

RESET lock_timeout;
