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
- `GET /api/users` - List all users (public)
- `GET /api/users/:id` - Get user by ID (public)
- `POST /api/users` - Create new user (requires auth)
- `PUT /api/users/:id` - Update user (requires auth)
- `DELETE /api/users/:id` - Delete user (requires auth)

### Topics
- `GET /api/topics` - List all topics (public, sorted by created_at desc)
- `GET /api/topics/:id` - Get topic by ID (public)
- `POST /api/topics` - Create new topic (requires auth)
- `PUT /api/topics/:id` - Update topic (requires auth)
- `DELETE /api/topics/:id` - Delete topic (requires auth)

### Posts
- `GET /api/posts` - List recent posts (public)
- `GET /api/topics/:id/posts` - List posts in a topic (public)
- `POST /api/posts` - Create new post (requires auth)
- `PUT /api/posts/:id` - Update post (requires auth)
- `DELETE /api/posts/:id` - Delete post (requires auth)

### Site Settings
- `GET /api/settings` - List all settings (public by default)
- `GET /api/settings/:key` - Get specific setting (public by default)
- `PUT /api/settings/:key` - Update setting value (requires auth)

## Development

- Run tests: `cargo test`
- Format code: `cargo fmt`
- Lint: `cargo clippy`

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

## Plugin Development

Routes can easily apply authentication using the provided macros:

```rust
use discourse_rs::{readable, writable, protected};

pub fn configure(cfg: &mut web::ServiceConfig) {
    // GET endpoints that respect require_auth_for_reads setting
    cfg.service(readable!(get_data, list_items));

    // POST/PUT/DELETE endpoints that always require auth
    cfg.service(writable!(create_item, delete_item));

    // Endpoints that always require auth (reads + writes)
    cfg.service(protected!(admin_endpoint));
}
```

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

### Phase 3: Polish (Next)
- [ ] Background jobs (backie with PostgreSQL queue)
- [ ] Username change propagation (update @mentions in posts)
- [ ] Pagination (limit/offset or cursor-based)
- [ ] Search (PostgreSQL full-text or Tantivy)
- [ ] Notifications
- [ ] Moderation tools (flags, hidden posts)
- [ ] Markdown rendering (raw -> cooked)
- [ ] Rate limiting
- [ ] API documentation (OpenAPI/Swagger)
- [ ] Guardian-style permissions (admin/moderator roles)
