CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    email VARCHAR(320) NOT NULL,
    username VARCHAR(50) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_login_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX users_email_lower_idx ON users (LOWER(email));
CREATE UNIQUE INDEX users_username_lower_idx ON users (LOWER(username));

CREATE TABLE posts (
    id BIGSERIAL PRIMARY KEY,
    author_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title VARCHAR(200) NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX posts_author_created_at_idx ON posts (author_id, created_at DESC);

CREATE TABLE post_shares (
    id BIGSERIAL PRIMARY KEY,
    post_id BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    shared_by_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    shared_with_user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT post_shares_not_self CHECK (shared_by_user_id <> shared_with_user_id),
    CONSTRAINT post_shares_unique_recipient UNIQUE (post_id, shared_with_user_id)
);

CREATE INDEX post_shares_recipient_post_idx ON post_shares (shared_with_user_id, post_id);

CREATE TABLE comments (
    id BIGSERIAL PRIMARY KEY,
    post_id BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX comments_post_created_at_idx ON comments (post_id, created_at);

CREATE TABLE votes (
    id BIGSERIAL PRIMARY KEY,
    post_id BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    value SMALLINT NOT NULL CHECK (value IN (-1, 1)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT votes_unique_user_per_post UNIQUE (post_id, user_id)
);

CREATE TABLE tags (
    id BIGSERIAL PRIMARY KEY,
    name VARCHAR(64) NOT NULL,
    slug VARCHAR(64) NOT NULL UNIQUE,
    CONSTRAINT tags_name_normalized CHECK (name = LOWER(name) AND name <> ''),
    CONSTRAINT tags_slug_normalized CHECK (slug = LOWER(slug) AND slug <> '')
);

CREATE UNIQUE INDEX tags_name_lower_idx ON tags (LOWER(name));

CREATE TABLE post_tags (
    post_id BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    tag_id BIGINT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (post_id, tag_id)
);

CREATE INDEX post_tags_tag_post_idx ON post_tags (tag_id, post_id);
