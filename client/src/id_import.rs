pub enum Error {
    /// A partial error for if any entries in don't have IDs.
    SomeKeysMissing(SomeMissingKeys),
    /// Error on parsing a bibliography string
    Parse(biblatex::ParseError),
}

pub struct SomeMissingKeys {
    missing_keys: Vec<(String, biblatex::RetrievalError)>,
    ids: Vec<String>,
}

impl std::fmt::Debug for SomeMissingKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.ids.is_empty() {
            write!(f, "successes aside, ")?;
        }
        write!(f, "these keys didn't have a DOI or URL: ")?;
        write!(
            f,
            "{}",
            self.missing_keys
                .iter()
                .map(|(key, err)| format!("{key} ({err:?})"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(err) => std::fmt::Debug::fmt(err, f),
            Error::SomeKeysMissing(err) => std::fmt::Debug::fmt(err, f),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for Error {}

impl SomeMissingKeys {
    /// Get the ids that were found without error.
    pub fn get_ids(&self) -> Vec<String> {
        self.ids.to_vec()
    }
}

/// Get either a DOI or URL from each BibTeX entry in the bibliography.
///
/// This will error if, in any case, the DOI is malformed or both the DOI
/// is missing and the URL is either missing or malformed.  When this
/// occurs, the successful ids can be recovered with [`SomeMissingKeys::get_ids`].
pub fn try_from_bibtex(bibtex_src: impl AsRef<str>) -> Result<Vec<String>, Error> {
    let bibliography = biblatex::Bibliography::parse(bibtex_src.as_ref()).map_err(Error::Parse)?;
    let maybe_ids = bibliography
        .iter()
        .map(|entry| match entry.doi() {
            Ok(doi) => Ok(doi),
            Err(err) => match err {
                biblatex::RetrievalError::TypeError(_) => Err((entry.key.clone(), err)),
                biblatex::RetrievalError::Missing(_) => {
                    entry.url().map_err(|e| (entry.key.clone(), e))
                }
            },
        })
        .collect::<Vec<_>>();
    if maybe_ids.iter().any(|id| id.is_err()) {
        let mut missing_keys = Vec::<(String, biblatex::RetrievalError)>::new();
        let mut ids = Vec::<String>::new();
        for maybe_id in maybe_ids {
            match maybe_id {
                Ok(id) => ids.push(id),
                Err(err) => missing_keys.push(err),
            };
        }
        return Err(Error::SomeKeysMissing(SomeMissingKeys {
            missing_keys,
            ids,
        }));
    }
    Ok(maybe_ids.into_iter().filter_map(|id| id.ok()).collect())
}
