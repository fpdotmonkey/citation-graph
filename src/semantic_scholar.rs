use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use serde::Deserialize;

const API_BATCH: &str = "https://api.semanticscholar.org/graph/v1/paper/batch";
const MAX_PAPERS_PER_BATCH_CALL: usize = 500;

#[derive(Debug, Clone)]
pub enum PaperId {
    Doi(String),
    SsId(String),
}

#[derive(Debug, Deserialize, Clone)]
struct ProtoPaper {
    #[serde(rename = "paperId")]
    id: Option<String>,
    title: String,
}

#[derive(Deserialize, Clone)]
pub struct Paper {
    title: String,
    url: String,
    #[serde(rename = "paperId")]
    id: String,
    references: Vec<ProtoPaper>,
}

impl Display for PaperId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PaperId::Doi(id) => write!(f, "DOI:{id}"),
            PaperId::SsId(id) => write!(f, "{id}"),
        }
    }
}

impl Paper {
    pub fn references(&self) -> Vec<ProtoPaper> {
        self.references
    }
}

pub async fn get_paper_batch(paper_ids: Vec<PaperId>) -> reqwest::Result<Vec<Paper>> {
    let client = reqwest::Client::new();
    let params = [("fields", "title,url,references")];
    let mut papers = Vec::<Paper>::new();
    for i in 0..std::cmp::max(papers.len() / MAX_PAPERS_PER_BATCH_CALL, 1) {
        eprintln!(
            "POST /graph/v1/paper/batch: {} papers",
            std::cmp::min(
                i * MAX_PAPERS_PER_BATCH_CALL + MAX_PAPERS_PER_BATCH_CALL,
                papers.len(),
            ) - i * MAX_PAPERS_PER_BATCH_CALL
        );
        let mut ids = HashMap::<&str, Vec<String>>::new();
        ids.insert(
            "ids",
            paper_ids[i * MAX_PAPERS_PER_BATCH_CALL
                ..std::cmp::min(
                    i * MAX_PAPERS_PER_BATCH_CALL + MAX_PAPERS_PER_BATCH_CALL,
                    papers.len(),
                )]
                .iter()
                .map(|id| id.to_string())
                .collect(),
        );

        let Ok(almost_papers) = client
            .post(API_BATCH)
            .json(&ids)
            .query(&params)
            .send()
            .await?
            .json::<Vec<Option<Paper>>>()
            .await
        else {
            let text = client
                .post(API_BATCH)
                .json(&ids)
                .query(&params)
                .send()
                .await?
                .text()
                .await?;
            println!("{text}");
            panic!("failed json serialization");
        };
        papers.extend(
            almost_papers
                .into_iter()
                .filter_map(|a| a)
                .collect::<Vec<Paper>>(),
        );
    }
    Ok(papers)
}
