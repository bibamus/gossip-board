DROP TABLE pending_signups;
DROP TABLE magic_link_tokens;

CREATE TABLE magic_link_tokens (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash CHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX magic_link_tokens_user_created_idx
    ON magic_link_tokens (user_id, created_at DESC);
CREATE INDEX magic_link_tokens_expiry_idx ON magic_link_tokens (expires_at);

ALTER TABLE users ADD COLUMN username_set BOOLEAN NOT NULL DEFAULT TRUE;