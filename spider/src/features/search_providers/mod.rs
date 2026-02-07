//! Search provider implementations.
//!
//! This module contains implementations of the [`super::search::SearchProvider`] trait
//! for various web search APIs.

#[cfg(feature = "search_bing")]
mod bing;
#[cfg(feature = "search_brave")]
mod brave;
#[cfg(feature = "search_serper")]
mod serper;
#[cfg(feature = "search_tavily")]
mod tavily;

#[cfg(feature = "search_bing")]
pub use bing::BingProvider;
#[cfg(feature = "search_brave")]
pub use brave::BraveProvider;
#[cfg(feature = "search_serper")]
pub use serper::SerperProvider;
#[cfg(feature = "search_tavily")]
pub use tavily::TavilyProvider;

pub use super::search::{SearchError, SearchOptions, SearchProvider, SearchResult, SearchResults};
