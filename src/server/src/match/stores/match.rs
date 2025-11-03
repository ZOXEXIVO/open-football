use async_compression::tokio::write::GzipEncoder;
use core::r#match::{MatchResult, ResultMatchPositionData};
use log::{debug, info};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MATCH_DIRECTORY: &str = "matches";
const CHUNK_DURATION_MS: u64 = 300_000; // 5 minutes per chunk

pub struct MatchStore;

impl MatchStore {
    pub async fn get(league_slug: &str, match_id: &str) -> Vec<u8> {
        let match_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}.json.gz", match_id));

        let mut file = File::options().read(true).open(&match_file).await.unwrap();

        let mut result = Vec::new();

        file.read_to_end(&mut result)
            .await
            .expect(format!("failed to read match {}", match_id).as_str());

        result
    }

    pub async fn get_chunk(league_slug: &str, match_id: &str, chunk_number: usize) -> Option<Vec<u8>> {
        let chunk_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}_chunk_{}.json.gz", match_id, chunk_number));

        let mut file = match File::options().read(true).open(&chunk_file).await {
            Ok(f) => f,
            Err(_) => {
                debug!("Chunk file not found: {}", chunk_file.display());
                return None;
            }
        };

        let mut result = Vec::new();
        file.read_to_end(&mut result)
            .await
            .expect(&format!("failed to read chunk {} for match {}", chunk_number, match_id));

        Some(result)
    }

    pub async fn get_metadata(league_slug: &str, match_id: &str) -> Option<serde_json::Value> {
        let metadata_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}_metadata.json", match_id));

        let mut file = match File::options().read(true).open(&metadata_file).await {
            Ok(f) => f,
            Err(_) => {
                debug!("Metadata file not found: {}", metadata_file.display());
                return None; // No metadata means no chunks available
            }
        };

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .await
            .expect(&format!("failed to read metadata for match {}", match_id));

        let metadata: serde_json::Value = serde_json::from_str(&contents)
            .expect("failed to parse metadata");

        Some(metadata)
    }

    pub async fn store(result: MatchResult) {
        let out_dir = PathBuf::from(MATCH_DIRECTORY).join(&result.league_slug);

        if let Ok(_) = tokio::fs::create_dir_all(&out_dir).await{}

        let out_file = out_dir.join(format!("{}.json.gz", result.id));

        let file = File::options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out_file)
            .await
            .expect(&format!("failed to create file {}", out_file.display()));

        let mut compressed_file = GzipEncoder::with_quality(file, async_compression::Level::Best);

        if let Some(res) = result.details {
            //serialize and write compressed data
            let file_data =
                serde_json::to_vec(&res.position_data).expect("failed to serialize data");

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

            // Store chunks
            Self::store_chunks(&result.league_slug, &result.id, &res.position_data).await;
        }
    }

    async fn store_chunks(league_slug: &str, match_id: &str, data: &ResultMatchPositionData) {
        let chunks = data.split_into_chunks(CHUNK_DURATION_MS);
        let chunk_count = chunks.len();

        info!("Storing {} chunks for match {}", chunk_count, match_id);

        // Store each chunk
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_file = PathBuf::from(MATCH_DIRECTORY)
                .join(league_slug)
                .join(format!("{}_chunk_{}.json.gz", match_id, idx));

            let file = File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&chunk_file)
                .await
                .expect(&format!("failed to create chunk file {}", chunk_file.display()));

            let mut compressed_file = GzipEncoder::with_quality(file, async_compression::Level::Best);

            let chunk_data = serde_json::to_vec(&chunk).expect("failed to serialize chunk");

            debug!("Chunk {} uncompressed size = {}", idx, chunk_data.len());

            compressed_file
                .write_all(&chunk_data)
                .await
                .expect("failed to write chunk data");

            compressed_file
                .shutdown()
                .await
                .expect("failed to shutdown chunk file");
        }

        // Store metadata
        let metadata_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}_metadata.json", match_id));

        let metadata = serde_json::json!({
            "chunk_count": chunk_count,
            "chunk_duration_ms": CHUNK_DURATION_MS,
            "total_duration_ms": data.max_timestamp()
        });

        tokio::fs::write(&metadata_file, serde_json::to_string_pretty(&metadata).unwrap())
            .await
            .expect(&format!("failed to write metadata file {}", metadata_file.display()));
    }
}
