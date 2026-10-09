DELETE FROM users WHERE username_set = FALSE;
ALTER TABLE users DROP COLUMN username_set;

DROP TABLE magic_link_tokens;

CREATE TABLE magic_link_tokens (
    id BIGSERIAL PRIMARY KEY,
    email VARCHAR(320) NOT NULL,
    token_hash CHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX magic_link_tokens_email_expiry_idx ON magic_link_tokens (email, expires_at);
CREATE INDEX magic_link_tokens_expiry_idx ON magic_link_tokens (expires_at);

CREATE TABLE pending_signups (
    id BIGSERIAL PRIMARY KEY,
    email VARCHAR(320) NOT NULL,
    signup_hash CHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX pending_signups_email_idx ON pending_signups (email);
CREATE INDEX pending_signups_expiry_idx ON pending_signups (expires_at);