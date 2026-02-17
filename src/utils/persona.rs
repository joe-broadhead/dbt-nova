/// Supported search personas used for ranking and boosts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchPersona {
    Default,
    Analyst,
    Engineer,
    Governance,
}

impl SearchPersona {
    /// Parse a persona name, defaulting when unrecognized.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        value.parse().unwrap_or(SearchPersona::Default)
    }

    /// Return the canonical lowercase persona label.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchPersona::Default => "default",
            SearchPersona::Analyst => "analyst",
            SearchPersona::Engineer => "engineer",
            SearchPersona::Governance => "governance",
        }
    }
}

impl std::str::FromStr for SearchPersona {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value.trim().to_lowercase().as_str() {
            "analyst" => SearchPersona::Analyst,
            "engineer" => SearchPersona::Engineer,
            "governance" => SearchPersona::Governance,
            _ => SearchPersona::Default,
        })
    }
}
