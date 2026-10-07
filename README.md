# Gossip Board

React + Vite frontend and Rust + Actix backend with PostgreSQL.

## Run with Docker

From the repository root:

```powershell
Copy-Item .env.example .env
docker compose up --build -d
```

Open <http://localhost:3000>. Compose starts PostgreSQL, waits for it to be
healthy, applies the embedded Diesel migrations on backend startup, and serves
the frontend through Nginx. Nginx proxies `/api` to the backend on the internal
Docker network, keeping authentication cookies on the same origin.

The database uses PostgreSQL 18 and mounts its volume at `/var/lib/postgresql`,
with version-specific data stored beneath it. Existing PostgreSQL 17 data
requires a supported major-version upgrade or dump/restore before reuse;
changing the image tag alone does not upgrade an existing database.

Without SMTP configuration, find the magic sign-in link in the backend logs:

```powershell
docker compose logs -f backend
```

Stop the stack with `docker compose down`. PostgreSQL data remains in the named
volume between runs. Database and backend ports are not published to the host;
only the frontend is exposed, on the loopback interface.

## Build images separately

```powershell
docker build -t gossip-board-backend:local .\backend
docker build -t gossip-board-frontend:local .\frontend
```

Both images use multi-stage builds and non-root runtime users. The backend
contains its migrations and requires `DATABASE_URL` at runtime. The frontend
expects a backend service named `backend` listening on port 8080 on the same
Docker network.

## Published images

The GitHub Actions workflow builds both images on pull requests to `main`
without pushing. Pushes to `main` publish `main`, `latest`, and `sha-<commit>` tags to
GitHub Container Registry:

```text
ghcr.io/bibamus/gossip-board/backend:latest
ghcr.io/bibamus/gossip-board/frontend:latest
```

Version tags such as `v1.0.0` publish matching image tags without replacing
`latest`. The workflow can also be run manually; it publishes only when run
against the default branch or a `v*` tag. Images target `linux/amd64` and builds
use separate GitHub Actions caches for each service.

Publishing uses the repository's `GITHUB_TOKEN` with `packages: write`; no
registry secrets need to be added. GHCR packages may initially be private.
Configure package visibility on GitHub for anonymous pulls, or authenticate
with a token that has `read:packages` for private packages.

## Configuration

Copy `.env.example` to `.env` to customize the Compose stack. Environment files
are ignored by Git and excluded from image build contexts.

| Variable | Purpose |
| --- | --- |
| `POSTGRES_PASSWORD` | Password for the bundled PostgreSQL database; change the local default before deployment. |
| `DATABASE_URL` | Optional override for an existing PostgreSQL database. |
| `FRONTEND_PORT` | Published frontend port, default `3000`. |
| `APP_BASE_URL` | Browser-facing frontend URL used in magic links, default `http://localhost:3000`. Update this when changing the port or hostname. |
| `APP_ENV` | Default `development`; `production` requires SMTP and defaults to secure cookies. |
| `COOKIE_SECURE` | Explicit cookie security override; use secure cookies with HTTPS. |
| `SMTP_HOST`, `SMTP_FROM` | SMTP server and sender address. |
| `SMTP_PORT` | SMTP port, default `587`. |
| `SMTP_TLS` | `none` for local plaintext SMTP, `starttls` for required STARTTLS (usually port `587`), or `implicit` for immediate TLS (usually port `465`). Defaults to `implicit` for compatibility; configure the port explicitly for your server. |
| `SMTP_USERNAME`, `SMTP_PASSWORD` | Optional SMTP credentials; set both together. |
| `BIND_ADDRESS` | Backend listener, default `127.0.0.1:8080` locally and `0.0.0.0:8080` in Docker. |

For the existing local test database, set this in `.env`:

```dotenv
DATABASE_URL=postgresql://postgres:password@host.docker.internal/postgres
```

`host.docker.internal` resolves to the host on Docker Desktop. PostgreSQL must
accept connections from Docker's network. On Linux, configure the corresponding
host gateway mapping if needed. Compose still starts its bundled database, but
the backend uses the override.

For deployment, configure SMTP, set `APP_ENV=production` and an HTTPS
`APP_BASE_URL`, and terminate TLS at a reverse proxy. The default Compose
configuration is intended for local development, not public exposure.

For running the backend without Docker, see [backend/README.md](backend/README.md).
