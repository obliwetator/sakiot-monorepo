-- Background jobs persist a stable, public error kind next to their error
-- text. Status endpoints derive the displayed message from the kind, so the
-- free-form `error` text that older releases filled with internal detail is
-- never shown again; rows without a trusted kind read back as unclassified.
--
-- Nullable, unconstrained, and additive so overlapping releases stay safe: an
-- older release keeps writing `error` alone (and reads the public message the
-- new release writes there), and a newer release may add kinds this one
-- reports as unclassified.
ALTER TABLE public.media_jobs ADD COLUMN error_kind text;
ALTER TABLE public.composition_jobs ADD COLUMN error_kind text;
ALTER TABLE public.recording_deletion_jobs ADD COLUMN error_kind text;
