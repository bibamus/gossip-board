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

Set `SMTP_TLS=none` for a local SMTP server without encryption,
`SMTP_TLS=starttls` for required STARTTLS (typically `SMTP_PORT=587`), or
`SMTP_TLS=implicit` for immediate TLS (typically `SMTP_PORT=465`). The default
is `implicit` to preserve existing behavior. `SMTP_PORT` remains `587` by
default regardless of TLS mode; set it to your server's actual port. Use a
TLS-enabled mode outside local development. TLS modes validate certificates
and do not fall back to plaintext.

The API provides `POST /api/auth/request-link`, `POST /api/auth/verify`,
`GET /api/auth/me`, and `POST /api/auth/logout`. Magic links expire after 15
minutes, are single-use, and are stored only as SHA-256 hashes. Successful
verification creates a 30-day server-side session with an HttpOnly,
SameSite=Lax cookie.
