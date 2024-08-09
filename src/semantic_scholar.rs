use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use serde::Deserialize;

const API_BATCH: &str = "https://api.semanticscholar.org/graph/v1/paper/batch";
const MAX_PAPERS_PER_BATCH_CALL: usize = 500;
// from https://www.crossref.org/blog/dois-and-matching-regular-expressions/
const DOI_REGEX: &str = r#"(?<id>10.\d{4,9}/[-._;()/:A-Z0-9]+)$"#;
const SEMANTIC_SCHOLAR_REGEX: &str =
    r#"^(https?://)?(www\.)?semanticscholar.org/paper/(?<id>[0-9a-f]+)$"#;

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
            return Ok(Self::Doi(caps["id"].to_string()));
        }
        if let Some(caps) = semantic_scholar_regex.captures(s) {
            return Ok(Self::SemanticScholar(caps["id"].to_string()));
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
    pub async fn get_paper_batch(&self, paper_ids: Vec<PaperId>) -> reqwest::Result<Vec<Paper>> {
        if paper_ids.is_empty() {
            eprintln!("no papers requested");
            return Ok(vec![]);
        }
        let params = [(
            "fields",
            "title,url,references.paperId,references.title,references.url",
        )];
        let mut papers = Vec::<Paper>::new();
        for i in 0..std::cmp::max(paper_ids.len() / MAX_PAPERS_PER_BATCH_CALL, 1) {
            eprintln!(
                "POST /graph/v1/paper/batch: {} papers",
                std::cmp::min((1 + i) * MAX_PAPERS_PER_BATCH_CALL, paper_ids.len())
                    - i * MAX_PAPERS_PER_BATCH_CALL
            );
            let mut ids = HashMap::<&str, Vec<String>>::new();
            ids.insert(
                "ids",
                paper_ids[i * MAX_PAPERS_PER_BATCH_CALL
                    ..std::cmp::min((1 + i) * MAX_PAPERS_PER_BATCH_CALL, paper_ids.len())]
                    .iter()
                    .map(|id| id.to_string())
                    .collect(),
            );

            let request = self
                .client
                .post(API_BATCH)
                .json(&ids)
                .query(&params)
                .send()
                .await
                .unwrap();
            let almost_papers = request.json::<Vec<Option<Paper>>>().await.unwrap();
            papers.extend(almost_papers.into_iter().flatten().collect::<Vec<Paper>>());
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
