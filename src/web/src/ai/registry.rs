use log::debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tokio::task::JoinSet;

use super::providers::AiProvider;
use core::ai::{AiService, CompletedAiRequest, PendingAiRequest};

pub struct AiProviderStats {
    pub request_count: AtomicU64,
    pub completed_count: AtomicU64,
    pub error_count: AtomicU64,
    pub total_duration_ms: AtomicU64,
}

impl AiProviderStats {
    fn new() -> Self {
        AiProviderStats {
            request_count: AtomicU64::new(0),
            completed_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            total_duration_ms: AtomicU64::new(0),
        }
    }
}

pub struct AiProviderEntry {
    pub id: u64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub model: String,
    pub batch_size: usize,
    pub provider: Box<dyn AiProvider>,
    pub stats: AiProviderStats,
}

#[derive(Clone)]
pub struct AiProviderInfo {
    pub id: u64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub model: String,
    pub batch_size: usize,
    pub request_count: u64,
    pub completed_count: u64,
    pub error_count: u64,
    pub avg_response_ms: u64,
}

pub struct AiProviderRegistry {
    providers: RwLock<Vec<AiProviderEntry>>,
    next_id: AtomicU64,
}

impl Default for AiProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AiProviderRegistry {
    pub fn new() -> Self {
        AiProviderRegistry {
            providers: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn add(&self, name: &str, provider: Box<dyn AiProvider>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let entry = AiProviderEntry {
            id,
            name: name.to_string(),
            host: provider.host().to_string(),
            port: provider.port(),
            model: provider.model().to_string(),
            batch_size: provider.batch_size(),
            provider,
            stats: AiProviderStats::new(),
        };
        self.providers.write().await.push(entry);
        id
    }

    pub async fn remove(&self, id: u64) -> bool {
        let mut providers = self.providers.write().await;
        let len_before = providers.len();
        providers.retain(|p| p.id != id);
        providers.len() < len_before
    }

    pub async fn list(&self) -> Vec<AiProviderInfo> {
        let providers = self.providers.read().await;
        providers
            .iter()
            .map(|p| {
                let request_count = p.stats.request_count.load(Ordering::Relaxed);
                let completed_count = p.stats.completed_count.load(Ordering::Relaxed);
                let total_duration_ms = p.stats.total_duration_ms.load(Ordering::Relaxed);
                let avg_response_ms = if completed_count > 0 {
                    total_duration_ms / completed_count
                } else {
                    0
                };
                AiProviderInfo {
                    id: p.id,
                    name: p.name.clone(),
                    host: p.host.clone(),
                    port: p.port,
                    model: p.model.clone(),
                    batch_size: p.batch_size,
                    request_count,
                    completed_count,
                    error_count: p.stats.error_count.load(Ordering::Relaxed),
                    avg_response_ms,
                }
            })
            .collect()
    }

    pub async fn total_request_count(&self) -> u64 {
        let providers = self.providers.read().await;
        providers
            .iter()
            .map(|p| p.stats.request_count.load(Ordering::Relaxed))
            .sum()
    }

    pub async fn total_completed_count(&self) -> u64 {
        let providers = self.providers.read().await;
        providers
            .iter()
            .map(|p| p.stats.completed_count.load(Ordering::Relaxed))
            .sum()
    }

    pub async fn provider_count(&self) -> usize {
        self.providers.read().await.len()
    }

    async fn execute_on_provider(
        &self,
        provider_index: usize,
        req: PendingAiRequest,
    ) -> CompletedAiRequest {
        let providers = self.providers.read().await;
        let response = if let Some(entry) = providers.get(provider_index) {
            let start = std::time::Instant::now();
            let result = match entry.provider.query(req.query, req.format).await {
                Ok(r) => Some(r),
                Err(e) => {
                    debug!(
                        "AI provider '{}' (id={}) failed: {}",
                        entry.name, entry.id, e
                    );
                    entry.stats.error_count.fetch_add(1, Ordering::Relaxed);
                    None
                }
            };
            let elapsed_ms = start.elapsed().as_millis() as u64;
            entry
                .stats
                .total_duration_ms
                .fetch_add(elapsed_ms, Ordering::Relaxed);
            entry.stats.completed_count.fetch_add(1, Ordering::Relaxed);
            result
        } else {
            None
        };

        CompletedAiRequest {
            club_id: req.club_id,
            priority: req.priority,
            response,
            handler: req.handler,
        }
    }
}

/// Implements core::AiService by delegating to the multi-provider registry.
/// All tokio / infrastructure concerns live here — core stays pure.
pub struct RegistryAiService {
    pub registry: Arc<AiProviderRegistry>,
}

impl AiService for RegistryAiService {
    fn is_enabled(&self) -> bool {
        self.registry
            .providers
            .try_read()
            .map(|p| !p.is_empty())
            .unwrap_or(true)
    }

    fn execute_batch(
        &self,
        requests: Vec<PendingAiRequest>,
    ) -> Pin<Box<dyn Future<Output = Vec<CompletedAiRequest>> + Send + '_>> {
        let registry = Arc::clone(&self.registry);
        Box::pin(async move {
            let providers = registry.providers.read().await;
            let provider_count = providers.len();
            if provider_count == 0 {
                return Vec::new();
            }

            let batch_sizes: Vec<usize> = providers.iter().map(|p| p.batch_size).collect();

            let total = requests.len();

            // Split requests across providers round-robin
            let mut per_provider: Vec<Vec<PendingAiRequest>> =
                (0..provider_count).map(|_| Vec::new()).collect();

            for (i, req) in requests.into_iter().enumerate() {
                per_provider[i % provider_count].push(req);
            }

            // Set request_count upfront so total is known immediately
            for (idx, batch) in per_provider.iter().enumerate() {
                if let Some(entry) = providers.get(idx) {
                    entry
                        .stats
                        .request_count
                        .fetch_add(batch.len() as u64, Ordering::Relaxed);
                }
            }
            drop(providers);

            debug!(
                "distributing {} requests across {} providers",
                total, provider_count
            );

            // Each provider runs its requests with batch_size concurrency limit
            let mut set = JoinSet::new();

            for (provider_idx, provider_requests) in per_provider.into_iter().enumerate() {
                let reg = Arc::clone(&registry);
                let batch_size = batch_sizes[provider_idx];
                set.spawn(async move {
                    execute_provider_batch(provider_idx, provider_requests, &reg, batch_size).await
                });
            }

            let mut completed = Vec::with_capacity(total);
            while let Some(Ok(results)) = set.join_next().await {
                completed.extend(results);
            }

            completed
        })
    }
}

async fn execute_provider_batch(
    provider_index: usize,
    requests: Vec<PendingAiRequest>,
    registry: &Arc<AiProviderRegistry>,
    batch_size: usize,
) -> Vec<CompletedAiRequest> {
    let mut results = Vec::with_capacity(requests.len());
    let mut iter = requests.into_iter();

    loop {
        let mut set = JoinSet::new();
        let mut spawned = 0;

        for req in iter.by_ref() {
            let reg = Arc::clone(registry);
            let idx = provider_index;
            set.spawn(async move { reg.execute_on_provider(idx, req).await });
            spawned += 1;
            if spawned >= batch_size {
                break;
            }
        }

        if spawned == 0 {
            break;
        }

        while let Some(Ok(completed)) = set.join_next().await {
            results.push(completed);
        }
    }

    results
}
