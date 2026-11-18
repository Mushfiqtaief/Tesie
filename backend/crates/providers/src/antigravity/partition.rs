use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tesie_core::{ProviderConnection, ProxyNode};
use tesie_proxy::MihomoProxyState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignedProxyBrief {
    pub id: String,
    pub name: String,
    pub proxy_type: String,
    pub ip: Option<String>,
    pub alive: bool,
    pub latency_ms: Option<i64>,
    pub is_direct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProxyPartition {
    pub connection_id: String,
    pub account_name: String,
    /// `"fallback"` (disjoint proxy pool slice) or `"direct"` (0 proxies configured/selected)
    pub mode: String,
    /// True only if there are fewer external proxies than enabled accounts (`0 < P < A`)
    pub shared_warning: bool,
    /// Primary sticky proxy currently serving this account
    pub primary_proxy: Option<AssignedProxyBrief>,
    /// Full assigned proxy slice for this account in deterministic order (`NODE - 1 .. NODE - 15`)
    pub assigned_proxies: Vec<AssignedProxyBrief>,
    /// Ordered healthy execution chain: `[sticky_primary, fallback_1, fallback_2, ...]`
    #[serde(skip)]
    pub healthy_fallback_chain: Vec<ProxyNode>,
}

fn to_brief(node: &ProxyNode, mihomo_states: &HashMap<String, MihomoProxyState>) -> AssignedProxyBrief {
    let (alive, latency_ms) = match mihomo_states.get(&node.name) {
        Some(st) => (st.alive, if st.alive { st.delay_ms.or(node.latency_ms) } else { None }),
        None => (node.is_direct || node.alive, node.latency_ms),
    };
    AssignedProxyBrief {
        id: node.id.clone(),
        name: node.name.clone(),
        proxy_type: node.proxy_type.clone(),
        ip: node.ip.clone(),
        alive,
        latency_ms,
        is_direct: node.is_direct,
    }
}

/// Orders external proxies so distinct exit IPs come first (in natural `NODE - 1..N` order),
/// followed by any secondary proxies sharing an exit IP. This guarantees that when `P >= A`,
/// different Antigravity accounts get genuinely distinct exit IPs first.
pub fn order_external_proxies_for_isolation(proxies: &[ProxyNode]) -> Vec<ProxyNode> {
    let mut seen_ips = HashSet::new();
    let mut unique_first = Vec::with_capacity(proxies.len());
    let mut duplicate_tail = Vec::new();

    for p in proxies {
        if p.is_direct {
            continue;
        }
        let ip_key = p.ip_key();
        if seen_ips.insert(ip_key) {
            unique_first.push(p.clone());
        } else {
            duplicate_tail.push(p.clone());
        }
    }
    unique_first.extend(duplicate_tail);
    unique_first
}

/// Computes smart, non-overlapping proxy partitions across Antigravity accounts:
/// - **Disjoint Distribution (`P >= A`)**: `P` proxies are split evenly across `A` accounts
///   (e.g. `2` accounts + `30` proxies => `15` dedicated proxies for Account 1 and `15` for Account 2).
/// - **Sticky Primary + Ordered Fallback (NO Round-Robin)**: Each account sticks to its primary
///   healthy proxy (`sticky_proxy_id`, or the first healthy proxy in its slice) and only moves
///   to the next proxy in its own slice if the current proxy fails health/dial checks.
/// - **No Proxies (`P == 0`)**: Gracefully routes via `DIRECT`.
pub fn compute_account_proxy_partitions(
    accounts: &[ProviderConnection],
    configured_proxies: &[ProxyNode],
    mihomo_states: &HashMap<String, MihomoProxyState>,
) -> Vec<AccountProxyPartition> {
    if accounts.is_empty() {
        return Vec::new();
    }

    let external = order_external_proxies_for_isolation(configured_proxies);
    let direct_node = configured_proxies
        .iter()
        .find(|p| p.is_direct)
        .cloned()
        .unwrap_or(ProxyNode {
            id: "direct".to_string(),
            name: "DIRECT".to_string(),
            proxy_type: "direct".to_string(),
            ip: None,
            host: None,
            port: None,
            username: None,
            password: None,
            latency_ms: None,
            alive: true,
            proxy_url: None,
            raw_outbound_json: None,
            is_direct: true,
            enabled: true,
            egress_ip: None,
            last_delay_ms: None,
            last_probed_at: None,
            consecutive_failures: 0,
            created_at: 0,
            updated_at: 0,
        });

    let account_count = accounts.len();
    let proxy_count = external.len();

    if proxy_count == 0 {
        let brief = to_brief(&direct_node, mihomo_states);
        return accounts
            .iter()
            .map(|acc| AccountProxyPartition {
                connection_id: acc.id.clone(),
                account_name: acc.name.clone(),
                mode: "direct".to_string(),
                shared_warning: false,
                primary_proxy: Some(brief.clone()),
                assigned_proxies: vec![brief.clone()],
                healthy_fallback_chain: vec![direct_node.clone()],
            })
            .collect();
    }

    let mut out = Vec::with_capacity(account_count);
    let mut offset = 0usize;
    let base = proxy_count / account_count;
    let rem = proxy_count % account_count;

    for (idx, acc) in accounts.iter().enumerate() {
        let (slice, shared_warning): (Vec<ProxyNode>, bool) = if proxy_count >= account_count {
            let take = base + usize::from(idx < rem);
            let sub = external[offset..offset + take].to_vec();
            offset += take;
            (sub, false)
        } else {
            // Fewer proxies than accounts: assign deterministically and flag shared_warning
            (vec![external[idx % proxy_count].clone()], idx >= proxy_count)
        };

        let is_alive = |p: &ProxyNode| -> bool {
            mihomo_states
                .get(&p.name)
                .map(|st| st.alive)
                .unwrap_or(p.alive)
        };

        let mut healthy_chain: Vec<ProxyNode> = Vec::new();
        // Keep sticky primary first when it belongs to this account's slice and is healthy in Mihomo
        if let Some(sticky_id) = acc.sticky_proxy_id.as_deref().or(acc.proxy_node_id.as_deref())
            && let Some(sticky_node) = slice.iter().find(|p| p.id == sticky_id && is_alive(p))
        {
            healthy_chain.push(sticky_node.clone());
        }
        for p in &slice {
            if is_alive(p) && !healthy_chain.iter().any(|h| h.id == p.id) {
                healthy_chain.push(p.clone());
            }
        }

        let assigned_briefs: Vec<AssignedProxyBrief> =
            slice.iter().map(|p| to_brief(p, mihomo_states)).collect();
        let primary_brief = healthy_chain
            .first()
            .map(|p| to_brief(p, mihomo_states))
            .or_else(|| assigned_briefs.first().cloned());

        out.push(AccountProxyPartition {
            connection_id: acc.id.clone(),
            account_name: acc.name.clone(),
            mode: "fallback".to_string(),
            shared_warning,
            primary_proxy: primary_brief,
            assigned_proxies: assigned_briefs,
            healthy_fallback_chain: healthy_chain,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_account(id: &str, sticky: Option<&str>) -> ProviderConnection {
        ProviderConnection {
            id: id.into(),
            provider_id: "antigravity".into(),
            name: format!("acc-{id}"),
            email: Some(format!("{id}@example.com")),
            api_key: Some("tok".into()),
            auth_data_json: None,
            quota_json: None,
            proxy_node_id: None,
            sticky_proxy_id: sticky.map(str::to_string),
            enabled: true,
            cooldown_until: None,
            backoff_level: 0,
            last_error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn mock_proxy(n: usize) -> ProxyNode {
        ProxyNode {
            id: format!("id-{n}"),
            name: format!("NODE - {n}"),
            proxy_type: "socks5".into(),
            ip: Some(format!("198.51.100.{n}")),
            host: Some(format!("198.51.100.{n}")),
            port: Some(1080),
            username: None,
            password: None,
            latency_ms: Some(40),
            alive: true,
            proxy_url: Some(format!("socks5://198.51.100.{n}:1080")),
            raw_outbound_json: None,
            is_direct: false,
            enabled: true,
            egress_ip: Some(format!("198.51.100.{n}")),
            last_delay_ms: Some(40),
            last_probed_at: None,
            consecutive_failures: 0,
            created_at: n as i64,
            updated_at: n as i64,
        }
    }

    #[test]
    fn splits_30_proxies_15_and_15_across_2_accounts_with_sticky_fallback() {
        let accounts = vec![mock_account("a1", None), mock_account("a2", None)];
        let proxies: Vec<ProxyNode> = (1..=30).map(mock_proxy).collect();
        let mut states = HashMap::new();
        for n in 1..=30 {
            states.insert(
                format!("NODE - {n}"),
                MihomoProxyState {
                    name: format!("NODE - {n}"),
                    proxy_type: "socks5".into(),
                    alive: true,
                    delay_ms: Some(35),
                },
            );
        }

        let parts = compute_account_proxy_partitions(&accounts, &proxies, &states);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].assigned_proxies.len(), 15);
        assert_eq!(parts[1].assigned_proxies.len(), 15);
        assert!(!parts[0].shared_warning);
        assert!(!parts[1].shared_warning);

        // Zero overlap between Account 1 (NODE - 1..15) and Account 2 (NODE - 16..30)
        let set1: HashSet<_> = parts[0].assigned_proxies.iter().map(|p| &p.name).collect();
        let set2: HashSet<_> = parts[1].assigned_proxies.iter().map(|p| &p.name).collect();
        assert!(set1.is_disjoint(&set2));
        assert_eq!(parts[0].primary_proxy.as_ref().unwrap().name, "NODE - 1");
        assert_eq!(parts[1].primary_proxy.as_ref().unwrap().name, "NODE - 16");

        // When NODE - 1 fails Mihomo health check, Account 1 falls back to NODE - 2 while Account 2 stays on NODE - 16
        states.get_mut("NODE - 1").unwrap().alive = false;
        let failover_parts = compute_account_proxy_partitions(&accounts, &proxies, &states);
        assert_eq!(failover_parts[0].primary_proxy.as_ref().unwrap().name, "NODE - 2");
        assert_eq!(failover_parts[1].primary_proxy.as_ref().unwrap().name, "NODE - 16");
    }
}
