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

use std::collections::HashMap;

use leaky_bucket::RateLimiter;
use rocket::{
    http::{Status, StatusClass},
    response::content::RawJson,
    serde::json::Json,
    tokio::time,
    State,
};

use endpoints::PAPER_BATCH;

const ENV_API_KEY: &str = "API_KEY";
const HEADER_API_KEY: &str = "x-api-key";

const SEMANTIC_SCHOLAR_BASE_URI: &str = "https://api.semanticscholar.org";
// the rate limit is 1 req/s.  I'll slow it by a little for safety.
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

async fn s2_response(
    fields: &'_ str,
    ids: &HashMap<&'_ str, Vec<String>>,
    api_key: &String,
    client: &reqwest::Client,
) -> reqwest::Result<(Status, String)> {
    let response = client
        .post(format!("{}{}", SEMANTIC_SCHOLAR_BASE_URI, PAPER_BATCH))
        .header(HEADER_API_KEY, api_key)
        .query(&[("fields", fields)])
        .json(&ids)
        .send()
        .await?;
    let status_code = Status::new(response.status().as_u16());
    if matches!(
        status_code.class(),
        StatusClass::ClientError | StatusClass::ServerError
    ) {
        // no point in awaiting an invalid body
        return Ok((status_code, "".into()));
    }
    let body = response.text().await?;
    Ok((status_code, body))
}

// This will be offset to PAPER_BATCH when mounted
#[post("/?<fields>", data = "<ids>")]
async fn paper_batch(
    fields: &'_ str,
    ids: Json<HashMap<&'_ str, Vec<String>>>,
    api_key: &State<String>,
    limiter: &State<RateLimiter>,
    client: &State<reqwest::Client>,
) -> Result<RawJson<String>, Status> {
    limiter.acquire_one().await;
    let max_tries = 10;
    let mut tries = 0;
    let ids = ids.into_inner();
    while tries < max_tries {
        match s2_response(fields, &ids, api_key.inner(), client.inner()).await {
            Err(err) => {
                eprintln!("response error: {err:?}");
                return Err(Status::InternalServerError);
            }
            Ok((status, body)) => match status {
                // Status::Constant can't be a pattern because it has a
                // manual impl ParitalEq, instead of #[derive].
                status if status == Status::TooManyRequests => (), // try again
                status
                    if matches!(
                        status.class(),
                        StatusClass::ClientError | StatusClass::ServerError
                    ) =>
                {
                    return Err(status)
                }
                _ => return Ok(RawJson(body)),
            },
        }
        tries += 1;
    }
    Err(Status::GatewayTimeout)
}

#[rocket::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = match std::env::var(ENV_API_KEY) {
        Ok(empty) if empty == *"" => Err(ApiKeyMissing {}),
        Ok(key) => Ok(key),
        Err(_) => Err(ApiKeyMissing {}),
    }?;
    let request_client = reqwest::Client::new();
    rocket::build()
        .manage(api_key)
        .manage(
            RateLimiter::builder()
                .initial(0)
                .max(RATE_LIMIT_COUNT)
                .interval(RATE_LIMIT_PERIOD)
                .build(),
        )
        .manage(request_client)
        .mount(PAPER_BATCH, routes![paper_batch])
        .ignite()
        .await?
        .launch()
        .await?;
    Ok(())
}
