use async_compression::tokio::write::GzipEncoder;
use core::r#match::MatchResult;
use log::{debug, info};
use std::fmt::format;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK_SIZE: usize = 8 * 1024;
const MATCH_DIRECTORY: &str = "matches";

pub struct MatchStore;

impl MatchStore {
    pub async fn get(league_slug: &str, match_id: &str) -> Vec<u8> {
        let match_file = format!("{}/{}/{}.json.gz", MATCH_DIRECTORY, league_slug, match_id);

        let mut file = File::options().read(true).open(match_file).await.unwrap();

        let mut result = Vec::new();

        file.read_to_end(&mut result)
            .await
            .expect(format!("failed to read match {}", match_id).as_str());

        result
    }

    pub async fn store(result: MatchResult) {
        let out_dir = format!("{}/{}", MATCH_DIRECTORY, result.league_slug);

        if let Ok(_) = tokio::fs::create_dir_all(&out_dir).await{}

        let out_file = format!("{}/{}.json.gz", out_dir, result.id);

        let file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_file)
            .await
            .expect(&format!("failed to create file {}", &out_file));

        let mut compressed_file = GzipEncoder::with_quality(file, async_compression::Level::Best);

        if let Some(res) = result.details {
            //serialize and write compressed data
            let file_data =
                serde_json::to_vec(&res.position_data).expect("failed to serialize data");

            debug!("uncompressed size = {}", file_data.len());

            compressed_file
                .write_all(&file_data)
                .await
                .expect("failed to write data");
            compressed_file
                .write_all(b"\n")
                .await
                .expect("failed to write newline");

            compressed_file
                .shutdown()
                .await
                .expect("failed to shutdown file");
        }
    }
}
