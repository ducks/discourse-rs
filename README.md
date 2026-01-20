# Discourse-rs

A Discourse-inspired forum platform built in Rust.

## Stack

- **Web Framework**: Actix-web (async HTTP server)
- **ORM**: Diesel (type-safe query builder)
- **Database**: PostgreSQL
- **Validation**: validator crate
- **Serialization**: serde

## Setup

1. Enter the Nix shell:
```bash
nix-shell
```

2. Start PostgreSQL:
```bash
db_start
```

3. Run migrations:
```bash
diesel migration run
```

4. Start the server:
```bash
RUST_LOG=info cargo run
```

The server will be available at http://127.0.0.1:8080

## API Endpoints

### Core
- `GET /` - Welcome message
- `GET /health` - Health check

### Authentication
- `POST /api/auth/register` - Register new user (returns JWT token)
- `POST /api/auth/login` - Login existing user (returns JWT token)

### Users
- `GET /api/users` - List all users (public, paginated)
- `GET /api/users/:id` - Get user by ID (public)
- `POST /api/users` - Create new user (requires auth)
- `PUT /api/users/:id` - Update user (requires auth)
- `DELETE /api/users/:id` - Delete user (requires auth)

### Topics
- `GET /api/topics` - List all topics (public, paginated, sorted by created_at desc)
- `GET /api/topics/:id` - Get topic by ID (public)
- `POST /api/topics` - Create new topic (requires auth)
- `PUT /api/topics/:id` - Update topic (requires auth)
- `DELETE /api/topics/:id` - Delete topic (requires auth)

### Posts
- `GET /api/posts` - List recent posts (public, paginated)
- `GET /api/topics/:id/posts` - List posts in a topic (public, paginated)
- `POST /api/posts` - Create new post (requires auth)
- `PUT /api/posts/:id` - Update post (requires auth)
- `DELETE /api/posts/:id` - Delete post (requires auth)

### Categories
- `GET /api/categories` - List all categories (public, ordered by position)
- `GET /api/categories/:id` - Get category by ID (public)
- `POST /api/categories` - Create category (moderator only)
- `PUT /api/categories/:id` - Update category (moderator only)
- `DELETE /api/categories/:id` - Delete category (moderator only)

### Site Settings
- `GET /api/settings` - List all settings (public by default)
- `GET /api/settings/:key` - Get specific setting (public by default)
- `PUT /api/settings/:key` - Update setting value (requires auth)

### Search
- `GET /api/search?q=term&limit=20` - Full-text search across topics and posts

### Moderation
- `POST /api/moderation/topics/lock` - Lock a topic (moderator only)
- `POST /api/moderation/topics/unlock` - Unlock a topic (moderator only)
- `POST /api/moderation/topics/pin` - Pin a topic (moderator only)
- `POST /api/moderation/topics/unpin` - Unpin a topic (moderator only)
- `POST /api/moderation/topics/close` - Close a topic (moderator only)
- `POST /api/moderation/topics/open` - Open a topic (moderator only)
- `POST /api/moderation/posts/hide` - Hide a post (moderator only)
- `POST /api/moderation/posts/unhide` - Unhide a post (moderator only)
- `POST /api/moderation/posts/delete` - Delete a post (moderator only)
- `POST /api/moderation/users/suspend` - Suspend a user (moderator only)

## Development

- Run tests: `cargo test`
- Format code: `cargo fmt`
- Lint: `cargo clippy`

## Pagination

List endpoints support pagination via query parameters:

- `page` - Page number (default: 1, minimum: 1)
- `per_page` - Items per page (default: 30, minimum: 1, maximum: 100)

Example:

```bash
# Get first 10 users
curl http://127.0.0.1:8080/api/users?per_page=10

# Get page 2 with 20 users per page
curl http://127.0.0.1:8080/api/users?page=2&per_page=20

# Get first 5 topics
curl http://127.0.0.1:8080/api/topics?per_page=5
```

Paginated endpoints:
- `GET /api/users`
- `GET /api/topics`
- `GET /api/posts`
- `GET /api/topics/:id/posts`

## Authentication

The API uses JWT (JSON Web Tokens) for authentication. After registering or
logging in, include the token in the Authorization header:

```bash
curl -H "Authorization: Bearer YOUR_TOKEN_HERE" http://127.0.0.1:8080/api/posts
```

### Privacy Settings

By default, GET endpoints are public and write operations require
authentication. To make all endpoints require authentication (private forum),
update the `require_auth_for_reads` setting:

```bash
curl -X PUT http://127.0.0.1:8080/api/settings/require_auth_for_reads \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"value":"true"}'
```

## Rate Limiting

The API is rate limited to 60 requests per minute per IP address. When the
limit is exceeded, the server returns a 429 Too Many Requests response.

## Guardian Permissions

The API uses Guardian-style permission extractors for role-based access control.
Simply add a guard to your route handler and permission checks are automatic:

```rust
use crate::guardian::{ModeratorGuard, AdminGuard, StaffGuard};

// Only moderators can access
async fn lock_topic(pool: web::Data<DbPool>, guard: ModeratorGuard, ...) { }

// Only admins can access
async fn delete_user(pool: web::Data<DbPool>, guard: AdminGuard, ...) { }

// Staff (admin or moderator) can access
async fn view_logs(pool: web::Data<DbPool>, guard: StaffGuard, ...) { }
```

Available guards:
- `AuthenticatedUser` - Any logged-in user
- `ModeratorGuard` - Trust level 4, moderator flag, or admin
- `AdminGuard` - Admin flag only
- `StaffGuard` - Admin or moderator
- `TrustLevel1Guard` through `TrustLevel3Guard` - Minimum trust level

Guards automatically return 403 Forbidden if the user lacks permission.

## Roadmap

### Phase 1: Core Models ✅
- [x] Users
- [x] Topics
- [x] Posts
- [x] Categories

### Phase 2: Basic Features ✅
- [x] Create/read users
- [x] Create/read topics
- [x] Create/read posts
- [x] Update operations (PUT endpoints)
- [x] Delete operations (DELETE endpoints)
- [x] User authentication (JWT/sessions)
- [x] Configurable privacy settings
- [x] Plugin-friendly auth helpers

### Phase 3: Polish (In Progress)
- [x] Pagination (limit/offset)
- [x] Background jobs (PostgreSQL-backed queue with worker pool)
- [x] Search (PostgreSQL full-text search)
- [x] Moderation tools (lock/pin/close topics, hide/delete posts, suspend users)
- [x] Rate limiting (60 requests/min per IP)
- [x] Guardian-style permissions (admin/moderator/trust level guards)
- [ ] Username change propagation (update @mentions in posts)
- [ ] Notifications
- [ ] Markdown rendering (raw -> cooked)
- [ ] API documentation (OpenAPI/Swagger)
