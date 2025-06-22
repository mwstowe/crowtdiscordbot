use anyhow::Result;
use rand::seq::SliceRandom;
use tracing::{info, error};
use crate::google_search::{GoogleSearchClient, SearchResult};

pub struct NewsSearcher {
    search_client: GoogleSearchClient,
}

impl NewsSearcher {
    pub fn new() -> Self {
        Self {
            search_client: GoogleSearchClient::new(),
        }
    }
    
    // Get a random interesting news article
    pub async fn get_random_news(&self) -> Result<Option<SearchResult>> {
        // List of interesting news sources and topics to search for
        let search_queries = [
            "site:hackaday.com",
            "site:arstechnica.com",
            "site:theverge.com technology",
            "site:wired.com weird",
            "site:sciencedaily.com discovery",
            "site:newscientist.com unusual",
            "site:news.ycombinator.com",
            "site:techcrunch.com",
            "site:gizmodo.com weird technology",
            "site:boingboing.net",
        ];
        
        // Choose a random search query
        let query = search_queries.choose(&mut rand::thread_rng())
            .unwrap_or(&"interesting technology news");
            
        info!("Searching for news with query: {}", query);
        
        // Perform the search
        match self.search_client.search(query).await {
            Ok(result) => {
                info!("Got search result: {:?}", result.is_some());
                Ok(result)
            },
            Err(e) => {
                error!("Error searching for news: {:?}", e);
                Ok(None)
            }
        }
    }
}
