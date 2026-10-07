diesel::table! {
    users (id) {
        id -> BigInt,
        email -> Varchar,
        username -> Varchar,
        created_at -> Timestamptz,
        last_login_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    magic_link_tokens (id) {
        id -> BigInt,
        user_id -> BigInt,
        token_hash -> Bpchar,
        expires_at -> Timestamptz,
        used_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    sessions (id) {
        id -> BigInt,
        user_id -> BigInt,
        session_hash -> Bpchar,
        expires_at -> Timestamptz,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    posts (id) {
        id -> BigInt,
        author_id -> BigInt,
        title -> Varchar,
        body -> Text,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        image_data -> Nullable<Text>,
    }
}

diesel::table! {
    post_shares (id) {
        id -> BigInt,
        post_id -> BigInt,
        shared_by_user_id -> BigInt,
        shared_with_user_id -> BigInt,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    comments (id) {
        id -> BigInt,
        post_id -> BigInt,
        user_id -> BigInt,
        body -> Text,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    votes (id) {
        id -> BigInt,
        post_id -> BigInt,
        user_id -> BigInt,
        value -> SmallInt,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    tags (id) {
        id -> BigInt,
        name -> Varchar,
        slug -> Varchar,
    }
}

diesel::table! {
    post_tags (post_id, tag_id) {
        post_id -> BigInt,
        tag_id -> BigInt,
    }
}

diesel::joinable!(magic_link_tokens -> users (user_id));
diesel::joinable!(sessions -> users (user_id));
diesel::joinable!(posts -> users (author_id));
diesel::joinable!(post_shares -> posts (post_id));
diesel::joinable!(comments -> posts (post_id));
diesel::joinable!(comments -> users (user_id));
diesel::joinable!(votes -> posts (post_id));
diesel::joinable!(votes -> users (user_id));
diesel::joinable!(post_tags -> posts (post_id));
diesel::joinable!(post_tags -> tags (tag_id));

diesel::allow_tables_to_appear_in_same_query!(
    comments,
    magic_link_tokens,
    post_shares,
    post_tags,
    posts,
    sessions,
    tags,
    users,
    votes,
);
