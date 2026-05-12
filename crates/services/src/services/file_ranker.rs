use std::path::Path;

use db::models::repo::{SearchMatchType, SearchResult};

const BASE_MATCH_SCORE_FILENAME: i64 = 100;
const BASE_MATCH_SCORE_DIRNAME: i64 = 10;
const BASE_MATCH_SCORE_FULLPATH: i64 = 1;

#[derive(Clone, Default)]
pub struct FileRanker;

impl FileRanker {
    pub fn new() -> Self {
        Self
    }

    pub async fn rank_search_results(
        &self,
        _repo_path: &Path,
        results: Vec<SearchResult>,
    ) -> Vec<SearchResult> {
        let mut ranked = results;
        ranked.sort_by(|a, b| {
            let score_a = match_type_score(&a.match_type);
            let score_b = match_type_score(&b.match_type);
            score_b.cmp(&score_a)
        });
        ranked
    }
}

fn match_type_score(mt: &SearchMatchType) -> i64 {
    match mt {
        SearchMatchType::FileName => BASE_MATCH_SCORE_FILENAME,
        SearchMatchType::DirectoryName => BASE_MATCH_SCORE_DIRNAME,
        SearchMatchType::FullPath => BASE_MATCH_SCORE_FULLPATH,
    }
}
