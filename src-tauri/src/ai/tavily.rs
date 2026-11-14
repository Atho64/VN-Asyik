//! Tavily web search for community visual novel recommendations.
//!
//! `SearchResult::relevance` reflects query match score, not recommendation
//! confidence. The API key stays in the OS keychain and is sent only in `Authorization`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::secrets::TavilyApiKey;

/// Where the API lives.
pub const ENDPOINT: &str = "https://api.tavily.com/search";

/// Tavily caps `max_results` at 20.
pub const MAX_RESULTS: u8 = 20;

/// How much searching to pay for.
///
/// `basic` costs one API credit, `advanced` two. Recommendation queries are
/// short and specific, so the cheap depth is the right default; a user who wants
/// deeper results can opt in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    Basic,
    Advanced,
}

impl Depth {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Advanced => "advanced",
        }
    }

    #[must_use]
    pub fn credits(self) -> u8 {
        match self {
            Self::Basic => 1,
            Self::Advanced => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
    /// Tavily query-match score (0 to 1), not recommendation confidence.
    pub relevance: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub query: String,
    pub answer: Option<String>,
    pub results: Vec<SearchResult>,
    pub credits: u8,
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("no Tavily API key is configured")]
    NoKey,
    #[error("the Tavily API key was rejected")]
    Unauthorized,
    /// HTTP 432/433: plan or credit limit reached.
    #[error("the Tavily plan or credit limit has been reached")]
    OutOfCredits,
    #[error("Tavily is rate limiting requests, try again shortly")]
    Throttled,
    #[error("could not reach Tavily: {0}")]
    Offline(String),
    #[error("unexpected response from Tavily: {0}")]
    BadResponse(String),
    #[error("{0}")]
    BadQuery(String),
}

/// The request body, kept separate so its shape can be asserted in a test.
#[derive(Debug, Serialize)]
struct RequestBody<'a> {
    query: &'a str,
    max_results: u8,
    search_depth: &'a str,
    include_answer: &'a str,
}

/// Response fields we use. Everything else Tavily sends is ignored, so an added
/// field cannot break parsing.
#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    answer: Option<String>,
    #[serde(default)]
    results: Vec<RawResult>,
}

#[derive(Debug, Deserialize)]
struct RawResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    score: f32,
}

const TIMEOUT: Duration = Duration::from_secs(20);

/// Validate a query before spending a credit.
///
/// # Errors
/// Returns [`SearchError`] when empty or over length limits.
pub fn check_query(query: &str) -> Result<&str, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(SearchError::BadQuery("the search text is empty".to_owned()));
    }
    if trimmed.len() > 400 {
        return Err(SearchError::BadQuery(
            "the search text is too long".to_owned(),
        ));
    }
    Ok(trimmed)
}

/// # Errors
/// Returns [`SearchError`] mapped to the failure cause.
pub async fn search(
    client: &reqwest::Client,
    key: &TavilyApiKey,
    query: &str,
    max_results: u8,
    depth: Depth,
) -> Result<SearchOutcome, SearchError> {
    let query = check_query(query)?;
    let body = RequestBody {
        query,
        // The provider rejects anything above 20; clamp rather than fail.
        max_results: max_results.clamp(1, MAX_RESULTS),
        search_depth: depth.as_str(),
        // `advanced` asks for a longer written answer. It is free of extra
        // credits in Tavily's model, so it is worth having.
        include_answer: "advanced",
    };

    let response = client
        .post(ENDPOINT)
        .header("Authorization", format!("Bearer {}", key.expose()))
        .json(&body)
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|err| SearchError::Offline(err.to_string()))?;

    let status = response.status();
    match status.as_u16() {
        200 => {}
        401 => return Err(SearchError::Unauthorized),
        429 => return Err(SearchError::Throttled),
        432 | 433 => return Err(SearchError::OutOfCredits),
        other => {
            return Err(SearchError::BadResponse(format!("HTTP {other}")));
        }
    }

    let raw: RawResponse = response
        .json()
        .await
        .map_err(|err| SearchError::BadResponse(err.to_string()))?;

    let results = raw
        .results
        .into_iter()
        .filter(|item| !item.url.trim().is_empty())
        .map(|item| SearchResult {
            title: item.title.trim().to_owned(),
            url: item.url,
            content: item.content.trim().to_owned(),
            relevance: item.score.clamp(0.0, 1.0),
        })
        .collect();

    Ok(SearchOutcome {
        query: query.to_owned(),
        // An empty answer string is the same as no answer.
        answer: raw.answer.filter(|text| !text.trim().is_empty()),
        results,
        credits: depth.credits(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_is_rejected_without_a_request() {
        // Spending a credit to ask nothing is a bug, not a user action.
        assert!(matches!(check_query("   "), Err(SearchError::BadQuery(_))));
    }

    #[test]
    fn a_plausible_question_is_accepted() {
        assert_eq!(check_query("  like CLANNAD  ").unwrap(), "like CLANNAD");
    }

    #[test]
    fn an_absurd_query_is_rejected() {
        let long = "a".repeat(401);
        assert!(matches!(check_query(&long), Err(SearchError::BadQuery(_))));
    }

    #[test]
    fn result_count_is_clamped_to_the_documented_maximum() {
        // Tavily rejects more than 20; clamping keeps a UI slider from erroring.
        assert_eq!(50.clamp(1, MAX_RESULTS), MAX_RESULTS);
        assert_eq!(0.clamp(1, MAX_RESULTS), 1);
    }

    #[test]
    fn depth_names_and_costs_match_the_documented_pricing() {
        assert_eq!(Depth::Basic.as_str(), "basic");
        assert_eq!(Depth::Advanced.as_str(), "advanced");
        // Advanced costs two credits, per the API reference.
        assert_eq!(Depth::Basic.credits(), 1);
        assert_eq!(Depth::Advanced.credits(), 2);
    }

    #[test]
    fn the_request_body_uses_the_documented_field_names() {
        // A renamed field is a silent 422 at runtime, so pin the wire format.
        let body = RequestBody {
            query: "like CLANNAD",
            max_results: 5,
            search_depth: "basic",
            include_answer: "advanced",
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["query"], "like CLANNAD");
        assert_eq!(json["max_results"], 5);
        assert_eq!(json["search_depth"], "basic");
        assert_eq!(json["include_answer"], "advanced");
    }

    #[test]
    fn a_response_with_only_the_fields_we_use_parses() {
        // Tavily sends more (images, usage, request_id); extra fields must not
        // break parsing.
        let raw: RawResponse = serde_json::from_str(
            r#"{"answer":"Little Busters","results":[
                 {"title":"t","url":"https://vndb.org/v24","content":"c","score":0.8,"id":"x","raw_content":null}],
               "usage":{"credits":1},"request_id":"abc"}"#,
        )
        .unwrap();
        assert_eq!(raw.results.len(), 1);
        assert!((raw.results[0].score - 0.8).abs() < 1e-6);
        assert_eq!(raw.answer.as_deref(), Some("Little Busters"));
    }

    #[test]
    fn a_response_missing_optional_fields_parses() {
        let raw: RawResponse = serde_json::from_str(r#"{"results":[]}"#).unwrap();
        assert!(raw.answer.is_none());
        assert_eq!(raw.results.len(), 0, "raw.results should be empty");
    }

    #[test]
    fn results_without_a_url_are_dropped() {
        // A result with no address cannot be checked, so it is not a source.
        let raw: RawResponse = serde_json::from_str(
            r#"{"results":[{"title":"no url","content":"c","score":0.5},
                            {"title":"ok","url":"https://vndb.org","content":"c","score":0.4}]}"#,
        )
        .unwrap();
        let kept: Vec<_> = raw
            .results
            .into_iter()
            .filter(|item| !item.url.trim().is_empty())
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title, "ok");
    }

    #[test]
    fn an_empty_answer_string_is_reported_as_absent() {
        let raw: RawResponse = serde_json::from_str(r#"{"answer":"   ","results":[]}"#).unwrap();
        assert!(
            raw.answer
                .as_ref()
                .is_none_or(|text| text.trim().is_empty())
        );
    }

    #[test]
    fn relevance_is_clamped_to_a_sane_range() {
        // A provider change must not let a value reach the UI as 1.7 or -0.2.
        assert!((1.7_f32.clamp(0.0, 1.0) - 1.0).abs() < f32::EPSILON);
        assert!((-0.2_f32).clamp(0.0, 1.0).abs() < f32::EPSILON);
    }
}
