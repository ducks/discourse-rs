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

### Users
- `GET /api/users` - List all users
- `GET /api/users/:id` - Get user by ID
- `POST /api/users` - Create new user

### Topics
- `GET /api/topics` - List all topics (sorted by created_at desc)
- `GET /api/topics/:id` - Get topic by ID
- `POST /api/topics` - Create new topic

### Posts
- `GET /api/posts` - List recent posts
- `GET /api/topics/:id/posts` - List posts in a topic
- `POST /api/posts` - Create new post

## Development

- Run tests: `cargo test`
- Format code: `cargo fmt`
- Lint: `cargo clippy`

## Roadmap

### Phase 1: Core Models ✅
- [x] Users
- [x] Topics
- [x] Posts
- [x] Categories

### Phase 2: Basic Features (In Progress)
- [x] Create/read users
- [x] Create/read topics
- [x] Create/read posts
- [ ] Update operations (PUT endpoints)
- [ ] Delete operations (DELETE endpoints)
- [ ] User authentication (JWT/sessions)
- [ ] Basic permissions (Guardian-style)

### Phase 3: Polish
- [ ] Background jobs (backie with PostgreSQL queue)
- [ ] Username change propagation (update @mentions in posts)
- [ ] Pagination (limit/offset or cursor-based)
- [ ] Search (PostgreSQL full-text or Tantivy)
- [ ] Notifications
- [ ] Moderation tools (flags, hidden posts)
- [ ] Markdown rendering (raw -> cooked)
- [ ] Rate limiting
- [ ] API documentation (OpenAPI/Swagger)
