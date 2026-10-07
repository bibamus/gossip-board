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
