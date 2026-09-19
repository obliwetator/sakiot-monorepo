-- OAuth identify is sufficient for the dashboard. Keep legacy columns during
-- rolling upgrades (an older web process may still write them), but never
-- retain the corresponding personal data.
ALTER TABLE discord_auth_user ALTER COLUMN discriminator SET DEFAULT '0';

CREATE FUNCTION scrub_discord_auth_identity() RETURNS trigger AS $$
BEGIN
    NEW.discriminator := '0';
    NEW.bot := NULL;
    NEW.system := NULL;
    NEW.mfa_enabled := NULL;
    NEW.banner := NULL;
    NEW.accent_color := NULL;
    NEW.locale := NULL;
    NEW.verified := NULL;
    NEW.email := NULL;
    NEW.flags := NULL;
    NEW.premium_type := NULL;
    NEW.public_flags := NULL;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER scrub_discord_auth_identity_before_write
BEFORE INSERT OR UPDATE ON discord_auth_user
FOR EACH ROW EXECUTE FUNCTION scrub_discord_auth_identity();

UPDATE discord_auth_user SET username = username;
