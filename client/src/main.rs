use std::collections::{HashMap, HashSet};

use argh::FromArgs;
use semantic_scholar::{Paper, PaperId, ProtoPaper, SemanticScholar};

mod id_import;
mod semantic_scholar;

#[derive(FromArgs)]
/// Generate a citation graph based on the contents of a bibliography.
pub struct Cli {
    /// the path to a Bib(La)TeX bibliography
    #[argh(positional)]
    bibliography: String,
    /// what URL will be serving the API
    #[argh(option, default = "\"api.fletcherporter.com/s2\".into()")]
    base_uri: String,
    /// how many search iterations should be performed
    #[argh(option, default = "4")]
    max_depth: usize,
    /// the citation density your bibliography's reference network.
    ///
    /// Informally, try to tune this so only some dozens of papers are
    /// searched in the last iteration.
    #[argh(option, default = "3.25")]
    connectivity: f64,
}

struct StagingData {
    citation_count: usize,
    paper: Paper,
}

type Staging = HashMap<String, StagingData>;

#[derive(Default, PartialEq, Eq, Hash)]
struct Reference {
    referencer: String,
    referencee: String,
}

type PaperList = HashSet<ProtoPaper>;
type ReferenceList = HashSet<Reference>;

impl Extend<Paper> for Staging {
    /// Use the Semantic Scholar ID as the key and set the citation count to 1.
    fn extend<I: IntoIterator<Item = Paper>>(&mut self, papers: I) {
        for paper in papers {
            let id = paper.id().to_owned();
            if let Some(staged) = self.insert(
                id.clone(),
                StagingData {
                    citation_count: 1,
                    paper,
                },
            ) {
                self.get_mut(&id).unwrap().citation_count = std::cmp::max(1, staged.citation_count);
            }
        }
    }
}

/// Escape `"` and replace `\` with `\\`.
fn escape<'a>(s: impl Into<&'a str>) -> String {
    s.into().replace('\\', "\\\\").replace('\"', "\\\"")
}

fn from_staging(staging: &Staging) -> PaperList {
    staging
        .iter()
        .map(|(_, data)| <Paper as Into<ProtoPaper>>::into(data.paper.clone()))
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli: Cli = argh::from_env();

    let paper_ids = match id_import::try_from_bibtex(std::fs::read_to_string(cli.bibliography)?) {
        Err(id_import::Error::SomeKeysMissing(err)) => {
            eprintln!("{err:?}; continuing anyway");
            Ok(err.get_ids())
        }
        other => other,
    }?;
    let paper_ids = semantic_scholar::parse_ids(paper_ids);

    let api = SemanticScholar::new(cli.base_uri);
    let mut staging = Staging::default();
    // one request before the loop to avoid creating a special cases
    staging.extend(api.get_paper_batch(paper_ids).await?);
    let mut paper_list = from_staging(&staging);
    let mut reference_list = ReferenceList::default();

    // And now the rest of the requests.
    for depth in 0..cli.max_depth {
        eprintln!("depth={depth}");
        let mut staged_paper_list = PaperList::default();
        let mut staged_reference_list = ReferenceList::default();
        let mut remove_staged = Vec::<String>::default();
        let mut batched_papers = Vec::<PaperId>::default();

        let minimum_citations = (depth as f64 * cli.connectivity.ln()).exp().floor() as usize;
        for (id, staged) in &staging {
            if staged.citation_count < minimum_citations {
                continue;
            }
            staged_reference_list.extend(
                staged
                    .paper
                    .references()
                    .iter()
                    .filter_map(|reference| reference.id())
                    .map(|ref_id| Reference {
                        referencer: id.clone(),
                        referencee: ref_id.to_string(),
                    }),
            );
            staged_paper_list.extend(
                staged
                    .paper
                    .references()
                    .iter()
                    .filter(|paper| paper.id().is_some())
                    .cloned(),
            );
            batched_papers.extend(
                staged
                    .paper
                    .references()
                    .iter()
                    .filter_map(|reference| reference.id())
                    .map(|id| PaperId::SemanticScholar(id.to_string())),
            );
            remove_staged.push(id.clone());
        }
        for id in remove_staged {
            staging.remove(&id);
        }
        let new_papers = api.get_paper_batch(batched_papers).await?;
        let new_papers_again = new_papers.clone();
        let reference_increments: Vec<_> = new_papers_again
            .iter()
            .flat_map(|paper| paper.references())
            .collect();
        staging.extend(new_papers);
        for reference in reference_increments {
            let Some(ref_id) = reference.id() else {
                continue;
            };
            if let Some((_id, staged)) = staging
                .iter_mut()
                .find(|(_id, staged)| staged.paper.id() == ref_id)
            {
                staged.citation_count += 1;
            }
        }
        reference_list.extend(staged_reference_list);
        paper_list.extend(staged_paper_list);
    }

    // pruning
    for _ in 0..10 {
        paper_list.retain(|paper| {
            !(reference_list
                .iter()
                .filter(|reference| Some(reference.referencee.clone()).as_deref() == paper.id())
                .count()
                <= 1
                && reference_list
                    .iter()
                    .filter(|reference| Some(reference.referencer.clone()).as_deref() == paper.id())
                    .count()
                    <= 1)
        });
        reference_list.retain(|reference| {
            paper_list
                .iter()
                .any(|paper| paper.id() == Some(reference.referencer.clone()).as_deref())
                && paper_list
                    .iter()
                    .any(|paper| paper.id() == Some(reference.referencee.clone()).as_deref())
        });
    }

    // make a DOT file
    println!("digraph {{");
    for paper in paper_list {
        println!(
            "    \"{}\" [label=\"{}\",URL=\"{}\"];",
            paper.id().expect("paper id"),
            escape(paper.title()),
            paper.url().unwrap_or_default(),
        );
    }
    for Reference {
        referencer,
        referencee,
    } in reference_list
    {
        println!("    {referencer:?} -> {referencee:?};");
    }
    println!("}}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_a_string() {
        assert_eq!(escape("asdf \"foo\" \\aaa"), "asdf \\\"foo\\\" \\\\aaa");
    }
}
