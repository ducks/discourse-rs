//! Category lookup by slug path and URL generation
//! (Category.find_by_slug_path_with_id, #slug_path, #url in
//! app/models/category.rb).

use sqlx::PgConnection;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Category {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub parent_category_id: Option<i32>,
    pub read_restricted: bool,
    pub description: Option<String>,
    pub default_view: Option<String>,
}

const COLUMNS: &str =
    "id, name, slug, color, parent_category_id, read_restricted, description, default_view";

impl Category {
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Category>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM categories WHERE id = $1"))
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// `Category.find_by_slug_path_with_id`: a trailing numeric segment is
    /// the id (the slugs only drive the redirect); otherwise the slugs are
    /// walked parent to child.
    pub async fn find_by_slug_path_with_id(
        conn: &mut PgConnection,
        path: &str,
        max_nesting: i64,
    ) -> Result<Option<Category>, sqlx::Error> {
        let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if let Some(last) = segments.last()
            && !last.is_empty()
            && last.bytes().all(|b| b.is_ascii_digit())
        {
            let id: i32 = match last.parse() {
                Ok(id) => id,
                Err(_) => return Ok(None),
            };
            segments.pop();
            return Self::find(conn, id).await;
        }
        Self::find_by_slug_path(conn, &segments, max_nesting).await
    }

    /// `Category.find_by_slug_path`: each segment lowercased, matched under
    /// the previous category; `<id>-category` slugs match by id.
    async fn find_by_slug_path(
        conn: &mut PgConnection,
        segments: &[&str],
        max_nesting: i64,
    ) -> Result<Option<Category>, sqlx::Error> {
        if segments.is_empty() || segments.len() as i64 > max_nesting {
            return Ok(None);
        }
        let mut parent: Option<i32> = None;
        let mut current: Option<Category> = None;
        for segment in segments {
            let slug = segment.to_lowercase();
            let by_id: Option<i32> = slug
                .strip_suffix("-category")
                .and_then(|id| id.parse().ok());
            let found: Option<Category> = sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM categories \
                 WHERE parent_category_id IS NOT DISTINCT FROM $1 AND (slug = $2 OR id = $3) \
                 ORDER BY id LIMIT 1"
            ))
            .bind(parent)
            .bind(&slug)
            .bind(by_id)
            .fetch_optional(&mut *conn)
            .await?;
            match found {
                Some(c) => {
                    parent = Some(c.id);
                    current = Some(c);
                }
                None => return Ok(None),
            }
        }
        Ok(current)
    }

    /// `slug_for_url`
    pub fn slug_for_url(&self) -> String {
        if self.slug.is_empty() {
            format!("{}-category", self.id)
        } else {
            self.slug.clone()
        }
    }

    /// `slug_path`: ancestors' slugs then its own.
    pub async fn slug_path(&self, conn: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
        let mut path = vec![self.slug_for_url()];
        let mut parent = self.parent_category_id;
        // Bounded walk: Discourse caps nesting at 3.
        for _ in 0..4 {
            let Some(id) = parent else { break };
            let Some(p) = Self::find(conn, id).await? else {
                break;
            };
            path.insert(0, p.slug_for_url());
            parent = p.parent_category_id;
        }
        Ok(path)
    }

    /// `full_slug("/")`: `parent/child/id`, what /c/ URLs carry.
    pub async fn full_slug(&self, conn: &mut PgConnection) -> Result<String, sqlx::Error> {
        let mut path = self.slug_path(conn).await?;
        path.push(self.id.to_string());
        Ok(path.join("/"))
    }

    /// `Category#url`
    pub async fn url(
        &self,
        conn: &mut PgConnection,
        base_path: &str,
    ) -> Result<String, sqlx::Error> {
        Ok(format!("{base_path}/c/{}", self.full_slug(conn).await?))
    }

    /// Direct subcategories visible to anonymous users, by position.
    pub async fn visible_subcategories(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Vec<Category>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM categories WHERE parent_category_id = $1 AND NOT read_restricted \
             ORDER BY position, id"
        ))
        .bind(self.id)
        .fetch_all(conn)
        .await
    }
}
