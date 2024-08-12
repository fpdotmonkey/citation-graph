//! My API key for Semantic Scholar has certain rate limits.  In order
//! to enforce these limits and to not share my secret key, I need to
//! have this server between the client and Semantic Scholar to enforce
//! those rate limits.
//!
//! The rate limits are
//! - 1 request/sec for /paper/batch, /paper/search, /recommendations,
//! - 10 request/sec for everything else.

#[macro_use]
extern crate rocket;

use leaky_bucket::RateLimiter;
use rocket::{tokio::time, State};

use endpoints::PAPER_BATCH;

const ENV_API_KEY: &str = "API_KEY";

const SEMANTIC_SCHOLAR_BASE_URI: &str = "https://api.semanticscholar.org";
const RATE_LIMIT_PERIOD: time::Duration = time::Duration::from_millis(1000);
const RATE_LIMIT_COUNT: usize = 1;

struct ApiKeyMissing {}

impl std::fmt::Debug for ApiKeyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "you must set {ENV_API_KEY}=<semantic-scholar-api-key>")
    }
}

impl std::fmt::Display for ApiKeyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ApiKeyMissing {}

// I need this endpoint to be at PAPER_BATCH, but this macro demands a
// string literal, not even a &'static str will do, so this is set to /
// and is offset to my desired endpoint.
#[post("/")]
async fn paper_batch(limiter: &State<RateLimiter>) -> String {
    limiter.acquire_one().await;
    format!("{:?}\n", std::time::SystemTime::now())
}

#[rocket::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = match std::env::var(ENV_API_KEY) {
        Ok(empty) if empty == *"" => Err(ApiKeyMissing {}),
        Ok(key) => Ok(key),
        Err(_) => Err(ApiKeyMissing {}),
    }?;
    rocket::build()
        .manage(api_key)
        .manage(
            RateLimiter::builder()
                .initial(RATE_LIMIT_COUNT)
                .max(RATE_LIMIT_COUNT)
                .interval(RATE_LIMIT_PERIOD)
                .build(),
        )
        .mount(PAPER_BATCH, routes![paper_batch])
        .ignite()
        .await?
        .launch()
        .await?;
    Ok(())
}
