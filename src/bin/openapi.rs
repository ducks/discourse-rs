//! Outputs the OpenAPI spec as JSON to stdout.
//! Used for generating static documentation.

use discourse_rs::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() {
    println!("{}", ApiDoc::openapi().to_pretty_json().unwrap());
}
