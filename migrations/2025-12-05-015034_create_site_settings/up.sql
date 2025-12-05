CREATE TABLE site_settings (
    key VARCHAR PRIMARY KEY,
    value VARCHAR NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert default settings
INSERT INTO site_settings (key, value) VALUES
    ('require_auth_for_reads', 'false'),
    ('site_name', 'Discourse RS'),
    ('site_description', 'A forum built with Rust');
