use std::sync::atomic::{AtomicUsize, Ordering};

use rand::seq::SliceRandom;
use tesie_core::{normalize_routing_strategy, IpCandidate, ProviderConfig, Result};
use tesie_store::Store;

use crate::controller::MihomoController;

/// Health-aware multi-strategy selector (`fallback` | `round-robin` | `random`) for OpenCode Free per-IP routing.
/// Always checks live proxy health in TesieCore (`GET /proxies` `alive` state) as the single
/// source of truth and excludes queued/exhausted exit IPs until verified live.
#[derive(Debug)]
pub struct IpPoolSelector {
    store: Store,
    rr_cursor: AtomicUsize,
}

impl IpPoolSelector {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            rr_cursor: AtomicUsize::new(0),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub async fn select_candidates_for_config(
        &self,
        mihomo: &MihomoController,
        now_ms: i64,
        cfg: &ProviderConfig,
    ) -> Result<Vec<IpCandidate>> {
        let mihomo_states = mihomo.fetch_proxy_states().await?;

        let mut candidates = self
            .store
            .quotas()
            .select_healthy_candidates_for_config(now_ms, cfg, |proxy| {
                match mihomo_states.get(&proxy.name) {
                    Some(st) => st.alive,
                    None => proxy.is_direct,
                }
            })
            .await?;

        let len = candidates.len();
        if len > 1 {
            match normalize_routing_strategy(&cfg.routing_strategy, "round-robin").as_str() {
                "fallback" => {
                    // Keep deterministic /24-interleaved order (use primary healthy IP until exhausted/queued)
                }
                "random" => {
                    candidates.shuffle(&mut rand::thread_rng());
                }
                _ => {
                    let offset = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % len;
                    candidates.rotate_left(offset);
                }
            }
        }

        Ok(candidates)
    }

    pub async fn round_robin_candidates_for_config(
        &self,
        mihomo: &MihomoController,
        now_ms: i64,
        cfg: &ProviderConfig,
    ) -> Result<Vec<IpCandidate>> {
        self.select_candidates_for_config(mihomo, now_ms, cfg).await
    }

    pub async fn round_robin_candidates(
        &self,
        mihomo: &MihomoController,
        now_ms: i64,
        proxy_mode: &str,
    ) -> Result<Vec<IpCandidate>> {
        // Mihomo is the single source of truth for proxy health.
        let mihomo_states = mihomo.fetch_proxy_states().await?;

        let mut candidates = self
            .store
            .quotas()
            .select_healthy_candidates_for_mode(now_ms, proxy_mode, |proxy| {
                match mihomo_states.get(&proxy.name) {
                    Some(st) => st.alive,
                    None => proxy.is_direct,
                }
            })
            .await?;

        let len = candidates.len();
        if len > 1 {
            let offset = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % len;
            candidates.rotate_left(offset);
        }

        Ok(candidates)
    }
}
