use serde::Deserialize;

const DEFAULT_PAGE: i64 = 1;
const DEFAULT_PER_PAGE: i64 = 30;
const MAX_PER_PAGE: i64 = 100;

#[derive(Deserialize, Clone)]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

impl PaginationParams {
    pub fn page(&self) -> i64 {
        self.page.unwrap_or(DEFAULT_PAGE).max(1)
    }

    pub fn per_page(&self) -> i64 {
        self.per_page
            .unwrap_or(DEFAULT_PER_PAGE)
            .clamp(1, MAX_PER_PAGE)
    }

    pub fn offset(&self) -> i64 {
        (self.page() - 1) * self.per_page()
    }
}
