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

2. Copy the example environment file:
```bash
cp .env.example .env
```

3. Set up the database:
```bash
createdb discourse_rs_development
diesel setup
```

4. Run migrations:
```bash
diesel migration run
```

5. Start the server:
```bash
cargo run
```

The server will be available at http://127.0.0.1:8080

## API Endpoints

- `GET /` - Welcome message
- `GET /health` - Health check
- `GET /api/*` - API endpoints (to be implemented)

## Development

- Run tests: `cargo test`
- Format code: `cargo fmt`
- Lint: `cargo clippy`

## Roadmap

### Phase 1: Core Models
- [ ] Users
- [ ] Topics
- [ ] Posts
- [ ] Categories

### Phase 2: Basic Features
- [ ] User authentication
- [ ] Create/read/update topics
- [ ] Create/read posts
- [ ] Basic permissions

### Phase 3: Polish
- [ ] Pagination
- [ ] Search
- [ ] Notifications
- [ ] Moderation tools
