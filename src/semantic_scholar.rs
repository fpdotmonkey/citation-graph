use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use serde::Deserialize;
use tokio::task::JoinSet;
use tokio::time;

const API_BATCH: &str = "https://api.semanticscholar.org/graph/v1/paper/batch";
const API_KEY_HEADER: &str = "x-api-key";
const MAX_PAPERS_PER_BATCH_CALL: usize = 500;
const RATE_LIMIT_TIME_PER_REQUEST: time::Duration = time::Duration::from_millis(1000);

// from https://www.crossref.org/blog/dois-and-matching-regular-expressions/
const DOI_REGEX: &str = r#"(?<id>10.\d{4,9}/[-._;()/:A-Z0-9]+)$"#;
const SEMANTIC_SCHOLAR_REGEX: &str =
    r#"^(https?://)?(www\.)?semanticscholar.org/paper/(?<id>[0-9a-f]+)$"#;
const ID_CAPTURE: &str = "id";

const ENV_API_KEY: &str = "API_KEY";

#[derive(Default)]
pub struct SemanticScholar {
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub enum PaperId {
    Doi(String),
    SemanticScholar(String),
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct ProtoPaper {
    #[serde(rename = "paperId")]
    id: Option<String>,
    title: String,
    url: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct Paper {
    title: String,
    url: String,
    #[serde(rename = "paperId")]
    id: String,
    references: Vec<ProtoPaper>,
}

pub enum Error {
    Request(reqwest::Error),
    Join(tokio::task::JoinError),
    Serialization(serde_json::Error, String),
    ApiKeyNotSet,
    RateLimit,
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Request(err) => std::fmt::Debug::fmt(err, f),
            Error::Join(err) => std::fmt::Debug::fmt(err, f),
            Error::Serialization(err, text) => write!(f, "{text}\n{err:?}"),
            Error::ApiKeyNotSet => write!(
                f,
                "environment variable API_KEY=<semantic-scholar-api-key> needs to be set"
            ),
            Error::RateLimit => write!(f, "rate limited by Semantic Scholar"),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Request(err) => Some(err),
            Error::Join(err) => Some(err),
            Error::Serialization(err, _text) => Some(err),
            Error::ApiKeyNotSet => None,
        }
    }
}

impl Display for PaperId {
    /// Write out in the format the API expects for the ids.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PaperId::Doi(id) => write!(f, "DOI:{id}"),
            PaperId::SemanticScholar(id) => write!(f, "{id}"),
        }
    }
}

impl TryFrom<&str> for PaperId {
    type Error = ();

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let doi_regex = regex::Regex::new(DOI_REGEX).unwrap();
        let semantic_scholar_regex = regex::Regex::new(SEMANTIC_SCHOLAR_REGEX).unwrap();
        if let Some(caps) = doi_regex.captures(s) {
            return Ok(Self::Doi(caps[ID_CAPTURE].to_string()));
        }
        if let Some(caps) = semantic_scholar_regex.captures(s) {
            return Ok(Self::SemanticScholar(caps[ID_CAPTURE].to_string()));
        }
        Err(())
    }
}

impl Paper {
    pub fn references(&self) -> &[ProtoPaper] {
        &self.references
    }

    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub fn title(&self) -> &str {
        self.title.as_str()
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }
}

impl ProtoPaper {
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn title(&self) -> &str {
        self.title.as_str()
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

impl From<Paper> for ProtoPaper {
    fn from(paper: Paper) -> Self {
        Self {
            id: Some(paper.id),
            title: paper.title,
            url: Some(paper.url),
        }
    }
}

impl SemanticScholar {
    pub async fn get_paper_batch(&self, paper_ids: Vec<PaperId>) -> Result<Vec<Paper>, Error> {
        if paper_ids.is_empty() {
            eprintln!("no papers requested");
            return Ok(vec![]);
        }
        let api_key = match std::env::var(ENV_API_KEY) {
            Ok(key) if key == *"" => Err(Error::ApiKeyNotSet),
            Err(_) => Err(Error::ApiKeyNotSet),
            Ok(key) => Ok(key),
        }?;
        let params = [(
            "fields",
            "title,url,references.paperId,references.title,references.url",
        )];
        let mut requests = JoinSet::new();
        for i in 0..std::cmp::max(paper_ids.len() / MAX_PAPERS_PER_BATCH_CALL, 1) {
            let low_index = i * MAX_PAPERS_PER_BATCH_CALL;
            let high_index = std::cmp::min((1 + i) * MAX_PAPERS_PER_BATCH_CALL, paper_ids.len());
            eprintln!(
                "POST /graph/v1/paper/batch: {} papers",
                high_index - low_index
            );
            let mut ids = HashMap::<&str, Vec<String>>::new();
            ids.insert(
                "ids",
                paper_ids[low_index..high_index]
                    .iter()
                    .map(|id| id.to_string())
                    .collect(),
            );

            requests.spawn(
                self.client
                    .post(API_BATCH)
                    .header(API_KEY_HEADER, &api_key)
                    .json(&ids)
                    .query(&params)
                    .send(),
            );
        }
        let mut responses = JoinSet::new();
        while let Some(response) = requests.join_next().await {
            responses.spawn(
                response
                    .map_err(Error::Join)?
                    .map_err(|err| match err {
                        error if error.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) => {
                            Error::RateLimit
                        }
                        error => Error::Request(error),
                    })?
                    .text(),
            );
        }
        let mut papers = Vec::<Paper>::new();
        while let Some(paper_txt) = responses.join_next().await {
            let paper_txt = paper_txt.map_err(Error::Join)?.map_err(Error::Request)?;
            papers.extend(
                serde_json::from_str::<Vec<Option<Paper>>>(paper_txt.as_ref())
                    .map_err(|err| Error::Serialization(err, paper_txt))?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<Paper>>(),
            );
        }
        Ok(papers)
    }
}

pub fn parse_ids(ids: Vec<String>) -> Vec<PaperId> {
    ids.into_iter()
        .map(|id: String| id.as_str().try_into())
        .filter_map(|id: Result<PaperId, ()>| id.ok())
        .collect()
}
