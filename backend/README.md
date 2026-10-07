# Backend

The backend requires PostgreSQL and its client development library (`libpq`)
for Diesel. Set `DATABASE_URL` to a PostgreSQL connection URL before starting:

```powershell
$env:DATABASE_URL = "postgres://user:password@localhost/gossip_board"
cargo run
```

On startup, the backend creates its Diesel connection pool and applies any
pending migrations from `migrations/`. The core migration creates users, posts,
shares, comments, votes, tags, and post-tag associations, including their
foreign keys, uniqueness rules, and query indexes. The Actix application state
exposes the pool for database-backed handlers.

## Magic-link authentication

Run the Vite frontend with `npm run dev` from `frontend/`; its `/api` requests
are proxied to the backend on port 8080. Without `SMTP_HOST`, local development
prints the one-time sign-in URL to the backend console. To send email through
SMTP, configure `SMTP_HOST`, `SMTP_FROM`, and optionally `SMTP_PORT` (default
587), `SMTP_USERNAME`, and `SMTP_PASSWORD`. The username and password must be
set together. Set `APP_BASE_URL` to the deployed frontend URL and
`APP_ENV=production`; production requires SMTP and enables secure cookies by
default. `COOKIE_SECURE` can explicitly override the cookie setting.
Set `ADMIN_EMAILS` to a comma-separated list of administrator email addresses.
Only signed-in users with a matching email can access `GET /api/admin/overview`,
which returns all users and the total number of posts. Administrators can delete
other users with `DELETE /api/admin/users/{id}`; deletion cascades to that user's
posts, comments, votes, and shares. Administrators cannot delete themselves from
the overview.

Set `SMTP_TLS=none` for a local SMTP server without encryption,
`SMTP_TLS=starttls` for required STARTTLS (typically `SMTP_PORT=587`), or
`SMTP_TLS=implicit` for immediate TLS (typically `SMTP_PORT=465`). The default
is `implicit` to preserve existing behavior. `SMTP_PORT` remains `587` by
default regardless of TLS mode; set it to your server's actual port. Use a
TLS-enabled mode outside local development. TLS modes validate certificates
and do not fall back to plaintext.

The API provides `POST /api/auth/request-link`, `POST /api/auth/verify`,
`GET /api/auth/me`, `PUT /api/auth/me` to change the current username, and
`DELETE /api/auth/me` to delete the account. Username changes accept up to 50
letters, numbers, dots, hyphens, or underscores; names are case-insensitive and
must be unique. Deleting an account also permanently deletes its posts,
comments, votes, and shares. `POST /api/auth/logout` revokes the current
session. Magic links expire after 15 minutes, are single-use, and are stored
only as SHA-256 hashes. Successful verification creates a 30-day server-side
session with an HttpOnly, SameSite=Lax cookie.

## Posts

Authenticated users can create posts with `POST /api/posts` using a JSON
`title` and `body`, and can optionally include `image_data` as a PNG, JPEG, GIF,
or WebP data URL (maximum 3 MB). The web editor supports file selection,
drag-and-drop, and pasting an image. List posts visible to them with
`GET /api/posts`, fetch one visible post with `GET /api/posts/{id}`, update their own post with
`PUT /api/posts/{id}`, and delete their own post with `DELETE /api/posts/{id}`.
Titles are limited to 200 characters and both fields must contain non-whitespace
text. The list and detail endpoints include the owner’s posts and posts
explicitly shared with the current user; only the owner can edit or delete.
Once a post has been shared, its title and body cannot be edited, even by the owner.
Edit attempts return `409`; the owner can still delete the post.
Unauthenticated requests receive `401`, while inaccessible or missing posts
receive `404`.

Share a post with an existing account by sending `POST /api/posts/{id}/shares`
with a JSON `username`; sharing uses usernames only. Any user who can view the
post may share it onward. The post appears in the recipient's visible-post list
on their next feed refresh. Use
`GET /api/posts/{id}/shares` to see its recipients. Shares are unique per
post/recipient; a repeated share returns `409`, a recipient must already have
an account, and attempts to share with yourself return `400`.

The authenticated `GET /api/users/search?q=...` endpoint returns up to ten
username matches for autocomplete once the query contains at least two
characters. The current user is excluded from results. Emails are not used or
returned by the sharing endpoints.

## Comments and voting

Owners and explicit share recipients can read comments with
`GET /api/posts/{id}/comments` and add one with
`POST /api/posts/{id}/comments` using JSON `body`. Comments must contain
non-whitespace text, appear oldest first, and identify their author by username
without exposing email addresses.

Use `POST /api/posts/{id}/votes` or `PUT /api/posts/{id}/votes` with JSON
`value` set to `1` or `-1`. Both atomically create or replace the user's single
vote, returning `score` (the sum of votes) and `your_vote`. Repeating a vote
does not increase the score. `GET /api/posts/{id}` now includes those fields
and `comment_count` alongside the existing post fields.

`DELETE /api/posts/{id}/votes` removes only the current user's vote and returns
the updated `score` and `your_vote: null`. Clicking the selected upvote or
downvote button removes the vote; clicking the opposite button changes it.

All interaction endpoints require a valid session (`401`) and access to the
post (`404` for missing or inaccessible posts). Invalid comments and votes
return `400`. The detail view provides comment input, voting controls, and
a discussion refresh button.

Database interaction coverage can be run against a migrated test database
with `TEST_DATABASE_URL` set:

```powershell
cargo test database_interactions_enforce_access_and_replace_votes -- --ignored
```

Fixtures run inside a transaction and are rolled back.

## Topic tags

Create and update posts with an optional JSON `tags` array of topic names.
Topics accept ASCII letters, numbers, spaces, and hyphens, with a maximum
normalized length of 64 characters. Names are lowercased, whitespace and
hyphens are collapsed into spaces, and duplicates are removed. Slugs use
hyphens in place of spaces. Empty or invalid names return `400`.

Post creation, listing, detail, and update responses include a `tags` array
of `{ id, name, slug }`. `GET /api/posts?tag=office%20news` filters the
current user's accessible posts by topic without exposing other posts.
`GET /api/tags` lists topics attached to accessible posts for suggestions
and filtering. `POST /api/tags` with JSON `name` creates or retrieves a
normalized topic.
`GET /api/tags?catalog=true` provides the shared topic vocabulary for
autocomplete without returning post associations. The picker adds one topic
at a time, creates unknown topics, and provides a remove button on each chip.

`PUT /api/posts/{id}/tags` with JSON `tags` replaces the topic associations.
Only the original owner may change its topics, including after sharing.
Title/body edits remain blocked with `409`; inaccessible posts return `404`.
Supplying `tags: []` clears
topics; omitting `tags` from a post update preserves existing associations.
Post content and topic changes are transactional.

With `TEST_DATABASE_URL` set, run the transactional topic coverage with:

```powershell
cargo test topics_are_deduplicated_access_filtered_and_locked_after_sharing -- --ignored
```
