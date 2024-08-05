use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use slotmap::{new_key_type, SlotMap};

mod semantic_scholar;
use semantic_scholar::{get_paper_batch, PaperId};

const SEMANTIC_SCHOLAR_BATCH: &str = "https://api.semanticscholar.org/graph/v1/paper/batch";
const MAX_PAPERS_PER_CALL: usize = 500;

// #[derive(Debug, Deserialize, Clone)]
// struct Paper {
//     #[serde(rename = "paperId")]
//     id: String,
//     title: String,
//     url: String,
//     references: Vec<Reference>,
// }

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
struct ProtoPaper {
    #[serde(rename = "paperId")]
    id: Option<String>,
    title: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Reference {
    referencee: ProtoPaper,
    referencer: PaperNodeKey,
}

#[derive(Default, Clone)]
struct ReferenceList {
    list: HashSet<Reference>,
}

new_key_type! { struct PaperNodeKey; }

impl ReferenceList {
    fn iter(&self) -> std::collections::hash_set::Iter<Reference> {
        self.list.iter()
    }
}

impl Reference {
    fn referencee(&self) -> &ProtoPaper {
        &self.referencee
    }
}

impl Extend<Reference> for ReferenceList {
    fn extend<I: IntoIterator<Item = Reference>>(&mut self, references: I) {
        for reference in references {
            self.list.insert(reference);
        }
    }
}

/// Escape `"` and `\\` with `\\`.
fn escape(s: String) -> String {
    s.replace("\\", "\\\\").replace("\"", "\\\"")
}

/// How many citations does `paper` have?
fn reference_count(reference_list: ReferenceList, paper: PaperNodeKey) -> usize {
    reference_list
        .iter()
        .filter(|reference| reference.referencee() == paper)
        .count()
}

#[tokio::main]
async fn main() -> reqwest::Result<()> {
    let mut paper_list = SlotMap::<PaperNodeKey, semantic_scholar::Paper>::with_key();
    let mut reference_list = HashSet::<(PaperNodeKey, PaperNodeKey)>::new();

    let client = reqwest::Client::new();
    let params = [("fields", "title,url,references")];

    // one request before the loop to avoid creating a special cases
    let mut ids = HashMap::new();
    let paper_ids = vec![
        PaperId::Doi("10.1016/j.jterra.2024.100989".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100988".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100998".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100999".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100975".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100976".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100977".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100984".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100985".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100987".to_string()),
        PaperId::Doi("10.1016/j.jterra.2024.100986".to_string()),
    ];
    ids.insert("ids", paper_ids);
    let papers = get_paper_batch(paper_ids).await?;
    let mut staged_references = ReferenceList::default();
    for paper in papers {
        let referencer = paper_list.insert(paper.clone());
        staged_references.extend(paper.references().iter().map(|reference| StagedReference {
            referencee: reference.clone(),
            referencer,
        }));
    }

    // And now the rest of the requests.
    for depth in 0..3 {
        let paper_ids: Vec<_> = staged_references
            .iter()
            .filter_map(|reference| reference.reference.id.clone())
            .map(|id| PaperId::SsId(id))
            .collect();
        let mut papers = get_paper_batch(paper_ids).await?;
        let old_staged_references = staged_references.clone();
        staged_references.clear();
        for paper in papers {
            let Some(paper) = paper else {
                continue;
            };

            let maybe_key = paper_list.iter().find(|(_, other)| other.url == paper.url);
            let referencee_key = if maybe_key.is_some() {
                maybe_key.unwrap().0
            } else {
                let key = paper_list.insert(paper.clone());
                staged_references.extend(paper.references.iter().map(|referencee| {
                    StagedReference {
                        referencee: referencee.clone(),
                        referencer: key,
                    }
                }));
                key
            };
            if reference_count(reference_list, referencee_key) < depth {
                continue;
            }

            let referencer_key: PaperNodeKey = staged_references
                .iter()
                .find(|staged_reference| staged_reference.referencee.title == paper.title)
                .expect("missing referencer")
                .referencer;
            reference_list.insert((referencer_key, referencee_key));
        }
    }

    // only use those references that were cited enough
    for _ in 0..20 {
        paper_list.retain(|paper_key, _| {
            !(reference_list
                .iter()
                .filter(|(_, referencee)| *referencee == paper_key)
                .count()
                == 1
                && reference_list
                    .iter()
                    .filter(|(referencer, _)| *referencer == paper_key)
                    .count()
                    == 0)
        });
        reference_list.retain(|(referencer, referencee)| {
            paper_list.contains_key(*referencer) && paper_list.contains_key(*referencee)
        });
    }

    // make a DOT file
    println!("digraph {{");
    for (paper_key, paper) in paper_list {
        println!(
            "    \"{paper_key:?}\" [label=\"{}\",URL=\"{}\",id=\"{}\"];",
            escape(paper.title),
            paper.url,
            paper.id
        );
    }
    for (referencer, referencee) in reference_list {
        println!("    \"{referencer:?}\" -> \"{referencee:?}\";");
    }
    println!("}}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_a_string() {
        assert_eq!(
            escape("asdf \"foo\" \\aaa".into()),
            "asdf \\\"foo\\\" \\\\aaa"
        );
    }
}
