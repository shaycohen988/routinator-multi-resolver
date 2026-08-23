use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::future::join_all;
use hickory_resolver::config::{
    LookupIpStrategy, NameServerConfig, Protocol, ResolverConfig, ResolverOpts,
};
use hickory_resolver::TokioAsyncResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Upstream resolvers queried in parallel, one per geographic region.
///
/// For CDN-fronted publication points the resolver's location determines which
/// anycast PoP the CDN DNS directs it to, so querying resolvers from different
/// continents yields IPs from different PoPs and widens the candidate pool.
pub const UPSTREAMS: &[([u8; 4], u16, &str)] = &[
    ([8,   8,   8,   8  ], 53, "NA/Google"),
    ([208, 67,  222, 222], 53, "NA/OpenDNS"),
    ([1,   1,   1,   1  ], 53, "Global/Cloudflare"),
    ([9,   9,   9,   9  ], 53, "EU/Quad9"),
    ([94,  140, 14,  14 ], 53, "EU/AdGuard"),
    ([77,  88,  8,   8  ], 53, "EU-East/Yandex"),
    ([114, 114, 114, 114], 53, "APAC/114DNS"),
    ([223, 5,   5,   5  ], 53, "APAC/AliDNS"),
    ([119, 29,  29,  29 ], 53, "APAC/DNSPod"),
    ([101, 101, 101, 101], 53, "APAC/TWNIC"),
];

/// A DNS resolver that queries several well-known upstream resolvers in
/// parallel and returns the union of all returned A and AAAA records.
///
/// IPv4 addresses are placed before IPv6 in the returned list so that reqwest
/// tries IPv4 first; this avoids silent hangs on hosts that have AAAA records
/// but no working IPv6 routing.
pub struct MultiIpResolver {
    resolvers: Arc<Vec<TokioAsyncResolver>>,
}

impl MultiIpResolver {
    pub fn new() -> Self {
        let mut opts = ResolverOpts::default();
        opts.timeout = std::time::Duration::from_secs(3);
        opts.attempts = 2;
        opts.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;

        let resolvers = UPSTREAMS
            .iter()
            .map(|(octets, port, _region)| {
                let ip: IpAddr = (*octets).into();
                let mut cfg = ResolverConfig::new();
                cfg.add_name_server(NameServerConfig::new(
                    SocketAddr::new(ip, *port),
                    Protocol::Udp,
                ));
                TokioAsyncResolver::tokio(cfg, opts.clone())
            })
            .collect();

        MultiIpResolver {
            resolvers: Arc::new(resolvers),
        }
    }
}

impl Resolve for MultiIpResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolvers = self.resolvers.clone();
        let host = name.as_str().to_owned();

        Box::pin(async move {
            let queries = resolvers.iter().map(|r| {
                let h = host.clone();
                async move { r.lookup_ip(h.as_str()).await }
            });
            let results: Vec<_> = join_all(queries).await;

            // Collect results per resolver for dedup and logging.
            let per_resolver: Vec<Vec<IpAddr>> = results
                .iter()
                .map(|r| match r {
                    Ok(lookup) => lookup.iter().collect(),
                    Err(_) => vec![],
                })
                .collect();

            // Deduplicate, placing IPv4 before IPv6 so reqwest tries v4 first.
            let mut seen = HashSet::new();
            let mut addrs_v4: Vec<SocketAddr> = Vec::new();
            let mut addrs_v6: Vec<SocketAddr> = Vec::new();
            for ips in &per_resolver {
                for &ip in ips {
                    if seen.insert(ip) {
                        if ip.is_ipv4() {
                            addrs_v4.push(SocketAddr::new(ip, 0));
                        } else {
                            addrs_v6.push(SocketAddr::new(ip, 0));
                        }
                    }
                }
            }

            let addrs: Vec<SocketAddr> = addrs_v4.iter()
                .chain(addrs_v6.iter())
                .cloned()
                .collect();

            if addrs.is_empty() {
                return Err(
                    format!("all upstream resolvers failed for {host}").into()
                );
            }

            let per_counts: Vec<usize> =
                per_resolver.iter().map(|v| v.len()).collect();

            let resolver_breakdown = UPSTREAMS
                .iter()
                .zip(per_counts.iter())
                .map(|((_, _, region), n)| format!("{}→{}", region, n))
                .collect::<Vec<_>>()
                .join("  ");

            let ip_list = addrs
                .iter()
                .map(|a| a.ip().to_string())
                .collect::<Vec<_>>()
                .join(" ");

            log::info!(
                target: "dns_pool",
                "{}: union={} IPs (v4={} v6={})  {}  [{}]",
                host, addrs.len(), addrs_v4.len(), addrs_v6.len(),
                resolver_breakdown, ip_list
            );

            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}
