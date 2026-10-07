# Gossip Board Initial Implementation Plan

## 1. Project goal
Build a web application for sharing gossip posts among users, with access-controlled sharing, discussion, voting, topic tagging, and email-based magic-link authentication.

## 2. Architecture overview
- Frontend: React + Vite + TypeScript
- Backend: Rust + Actix-web
- Data layer: PostgreSQL via Diesel ORM with Diesel migrations
- Containerization: separate Dockerfiles for frontend and backend
- Auth model: email magic-link sign-in using short-lived tokens
- App pattern: backend API + frontend SPA, with server-rendered or client-side navigation handled by Vite

## 3. Core principles for the MVP
- Security first: every post action must validate user identity and access permissions
- Minimal but extensible schema: keep data model normalized enough for sharing, comments, voting, and filtering
- Clear phase-based delivery: start with auth and ownership, then add sharing and interactions, then polish with tags and Docker packaging
- Local-first validation: run backend migrations and frontend/backend builds locally before final packaging

## 4. Delivery sequence
This plan follows the dependency order needed to build a working MVP without introducing blocked work.

### Phase 1: App scaffold
Goal: create the initial frontend and backend project structure and establish a runnable local baseline.

Tasks:
- Initialize the React + Vite TypeScript frontend project
- Initialize the Rust project for Actix-web and Diesel
- Create a shared repo layout for frontend and backend code
- Configure a simple backend health endpoint and frontend placeholder page
- Establish local run commands for frontend and backend
- Decide any shared conventions: environment variables, config file layout, logging, and API base URL

Outputs:
- Frontend app boots locally
- Backend app boots locally and exposes a health check
- Developers can run both services together for development

Acceptance criteria:
- `npm install` and `cargo build` succeed in a clean environment
- Frontend loads without runtime errors
- Backend responds on a known local port

### Phase 2: Database schema and migrations
Goal: define the core data model that supports users, posts, sharing, comments, votes, and tags in PostgreSQL.

Core tables/entities:
- users
  - id
  - email
  - created_at
  - maybe `last_login_at` or provider metadata if needed for magic-link flow
- posts
  - id
  - author_id
  - title or subject
  - body/content
  - created_at
  - updated_at
  - maybe deleted_at or soft-delete field if needed
- post_shares
  - id
  - post_id
  - shared_by_user_id
  - shared_with_user_id
  - created_at
  - optional permission metadata if future extension is needed
- comments
  - id
  - post_id
  - user_id
  - body
  - created_at
  - updated_at
- votes
  - id
  - post_id
  - user_id
  - value (upvote/downvote)
  - created_at
  - unique constraint on (post_id, user_id) to prevent duplicate votes
- tags
  - id
  - name
  - normalized slug if needed
- post_tags
  - post_id
  - tag_id

Relationships and logic:
- A post has one owner/author
- A user can view a post if they are the author or were explicitly shared the post
- A share is a permission link between users and a post
- A comment belongs to a post and a user
- A vote is per user per post, with a signed value
- Tags are many-to-many with posts

Tasks:
- Define Diesel schema structs and table definitions
- Create migrations for all tables and indexes
- Add constraints for duplicate votes and possibly unique tag names
- Add database connection setup and app state wiring in backend

Outputs:
- PostgreSQL schema with all core tables in Diesel migrations
- Diesel models for repositories and query access

Acceptance criteria:
- Diesel migrations run cleanly on a fresh PostgreSQL database
- Key relationships can be queried efficiently
- Data integrity rules prevent duplicate votes and invalid tag assignments

### Phase 3: Magic-link authentication
Goal: allow users to sign in with their email address and receive a magic link.

Flow:
- User enters email in the frontend
- Backend validates email format
- Backend creates a one-time token or signed verification code
- Backend sends a link via email to the user
- User clicks link
- Backend validates token, identifies user, creates a session or token, and marks the user as authenticated
- Authenticated requests use cookie-based or bearer-token session management

Implementation details:
- Add a `users` table entry for each email if needed
- Use a secure signed token containing user id and expiry time
- Keep a short expiry window (for example 15–60 minutes)
- Use environment variables for SMTP configuration and app base URL
- Add login/logout endpoints to the backend API
- Frontend needs a login screen, “magic link sent” confirmation, and authenticated state handling

Tasks:
- Add auth routes in Actix-web
- Create email sending abstraction or mock SMTP adapter for local/dev testing
- Implement token generation/validation
- Implement session creation and middleware for protected routes
- Add frontend auth state and login form

Outputs:
- Working magic-link login flow end-to-end in development mode
- Protected API routes reject unauthenticated users

Acceptance criteria:
- User can sign in with email only
- Invalid or expired links reject authentication
- Logged-in user can access protected routes
- Logout clears the session and denies future access

### Phase 4: Post CRUD and access model
Goal: allow users to create, view, edit, and delete gossip posts.

User behaviors:
- Authenticated user creates own post
- User can view their own posts
- User can view posts shared with them
- User can edit only their own posts
- User can delete only their own posts
- Shared posts remain linked to original author and appear in recipient views

API actions:
- `POST /posts` create a post
- `GET /posts` list user-visible posts
- `GET /posts/{id}` fetch a post with details if authorized
- `PUT /posts/{id}` edit if owner
- `DELETE /posts/{id}` delete if owner

Backend rules:
- Check authentication at route layer
- Authorize by comparing current user id to post author id
- When listing posts, query all posts where user is author or exists in `post_shares` for that user

Frontend screens:
- Post list/dashboard
- Create post form
- Edit post form
- Detail view showing content and metadata

Tasks:
- Add backend post repository and handlers
- Add frontend pages and form components for post CRUD
- Add authorization guard logic
- Add UI states for empty states, loading, and errors

Outputs:
- Users can manage their own gossip posts
- Shared posts show up in the appropriate feed without creating duplicate ownership records

Acceptance criteria:
- Owner can create/edit/delete posts
- Non-owner cannot mutate a post
- Recipient can view shared posts without editing permissions

### Phase 5: Sharing and shared-content visibility
Goal: enable post sharing among users and support reopening access to other users.

Features:
- Share a post with another user by email or user id
- View all posts shared with the current user
- Re-share a post that was shared to the current user, as long as the user has access to it
- Prevent duplicate shares from creating conflicting permissions

Core logic:
- `post_shares` is the access ledger
- A user who received a post may share it onward to others if the original shared post is visible to them
- Shared posts remain tied to the source author but can be re-shared through a permission model

Potential rules to define early:
- If a user receives a post shared by someone else, can they share it without explicit approval? The requirements suggest yes, “Posts shared with you can be shared with others.”
- Use the same `post_shares` table to store each explicit share relation, which also allows access history

Tasks:
- Add share creation endpoint and UI
- Add share list or recipient selection flow
- Add post filtering to show owned vs. shared posts
- Add backend queries for “posts visible to user” and “transitive sharing rules” if needed

Outputs:
- User can share posts with any other users they can identify in the system
- Shared posts appear in recipients’ visible post list
- Shared posts can be further shared by recipients

Acceptance criteria:
- Share creates an access record
- Post appears in recipient feed immediately
- Re-share is allowed by the current viewer if access is valid

### Phase 6: Comments and voting
Goal: add interactive discussion and rating mechanisms for accessible posts.

Comment behavior:
- Users with access to a post can add comments
- Comments appear beneath the post in order of creation
- Owner and recipients may comment if they have access
- Non-authorized users cannot comment

Voting behavior:
- Users with access to a post can upvote or downvote
- One vote per user per post, with change allowed by replacing the previous vote
- Display aggregate score for each post

Backend operations:
- `POST /posts/{id}/comments`
- `GET /posts/{id}/comments`
- `POST /posts/{id}/votes`
- `PUT /posts/{id}/votes`
- `GET /posts/{id}` returns comment and vote summary

Frontend UI:
- Comment list and input
- Vote controls with score display
- Loading and validation states

Tasks:
- Add comment models and repository queries
- Add vote table and aggregate query logic
- Add route handlers and permission checks
- Add UI for comments and voting

Outputs:
- Posts can be discussed and ranked by the user community with valid access checks

Acceptance criteria:
- Only accessible users can comment and vote
- Vote count reflects upvote/downvote totals
- Duplicate voting is prevented or replaced correctly

### Phase 7: Topic tags and filtering
Goal: support categorizing posts and filtering the post list by topic.

Topics:
- Tags are used as topics for posts
- Users may add one or more tags to a post
- Feed can be filtered by selected tag/topic

Data model details:
- `tags` table has stable names and normalized values
- `post_tags` joins posts and tags
- Tag names should be unique and normalized to lowercase for consistent filtering

User functionality:
- Add tag selector while creating/editing posts
- Show tags on the post detail view and list cards
- Filter visible posts by topic
- Search or quick filtering by tag if useful in the MVP

Tasks:
- Add backend tag create/list association endpoints
- Add frontend tags form controls
- Add post listing filters by tag
- Add validation for duplicate tags and empty tag creation

Outputs:
- Posts show topic labels
- Users can browse by topic

Acceptance criteria:
- Post can be assigned multiple tags
- Filter by tag returns only matching accessible posts
- Tag names are consistent and deduplicated

### Phase 8: Docker packaging and deployment readiness
Goal: provide containerized builds for both frontend and backend so the app can be run consistently across environments.

Deliverables:
- Dockerfile for backend
- Dockerfile for frontend
- Compose configuration if helpful for local orchestration
- Environment variable documentation for database path, SMTP, secret keys, and frontend API URL

Tasks:
- Create backend Dockerfile using Rust build stage and small runtime image
- Create frontend Dockerfile using Node build and Nginx or static serve approach
- Ensure the backend can run against PostgreSQL and persist data in a managed database service or local Postgres container
- Test both container builds locally

Outputs:
- Docker images can be built successfully
- Application starts in containers for local testing

Acceptance criteria:
- `docker build` succeeds for frontend and backend
- App remains functional in a local containerized environment
- Database persistence and config variables work correctly

## 5. Suggested implementation order in practice
The recommended sequence is:
1. Scaffold frontend + backend
2. Define database schema and migrations
3. Build auth flow
4. Implement posts
5. Add sharing logic
6. Add comments and voting
7. Add topic tagging
8. Package with Docker

This order keeps the app building incrementally while reducing the chance of having to rework core domain logic later.

## 6. Risk areas to watch closely
- Magic-link email flow needs secure token handling and reliable local email testing
- Access-control logic must be centralized to prevent permission bugs across post list, detail, share, vote, and comment flows
- Vote uniqueness and share duplication should be enforced at the database layer to reduce inconsistent application state
- Tag normalization and filtering should be lightweight but consistent to avoid duplicate topics

## 7. MVP definition
The initial MVP is successful when the following are true:
- A user can sign in via email magic link
- They can create, edit, delete, and view gossip posts
- They can share posts with other users
- They can view posts shared by others
- They can comment and vote on accessible posts
- They can filter visible posts by topic
- The application is runnable via local dev commands and containerized builds

## 8. Verification checklist
Before considering the initial requirements done, verify:
- Auth works with email and magic link
- Posts are protected by ownership/access rules
- Sharing permissions are created and enforced
- Comments and votes are access-controlled
- Tag filtering works across visible posts
- Diesel migrations support a fresh database startup
- Docker builds succeed for both services
