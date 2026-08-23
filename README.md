# RobustRP

A fork of [Routinator](https://github.com/NLnetLabs/routinator) (v0.15.2) that improves RPKI fetch resilience for CDN-fronted publication points by querying ten geographically distributed DNS resolvers in parallel and presenting the full union of returned IPs as failover candidates.

## Background

RPKI Relying Parties (RPs) fetch signed objects from publication points (PPs) over RRDP (HTTP). Many major PPs such as RIPE NCC are fronted by DNS geo-routing CDNs. These CDNs return the IP of the nearest Point of Presence (PoP) based on the querying resolver's location, so a standard RP using a single system resolver only ever sees one PoP's IPs. If that PoP becomes unreachable, the fetch fails even though other PoPs are healthy.
Failure to retrieve updated RPKI objects creates a significant security issue, potentially exposing the network to BGP hijacks.

RobustRP queries resolvers from five geographic regions simultaneously, over both A and AAAA records. Because each resolver is steered to a different PoP, the union pool spans multiple PoPs across both address families. If a connection to one IP fails, the RP automatically retries the next candidate, providing transparent failover across PoPs.

## What Changed

Roughly 145 lines across four files — no changes to validation logic, TAL handling, manifest verification, or VRP output:

| File | Change |
|---|---|
| `src/collector/rrdp/dns.rs` | New file — `MultiIpResolver` implementation |
| `src/collector/rrdp/http.rs` | Hook resolver into reqwest client builder |
| `src/collector/rrdp/mod.rs` | Register `dns` module |
| `Cargo.toml` | Add `hickory-resolver` dependency |

### Upstream resolvers

| Region | Resolver | Address |
|---|---|---|
| North America | Google | 8.8.8.8 |
| North America | OpenDNS | 208.67.222.222 |
| Global | Cloudflare | 1.1.1.1 |
| Europe | Quad9 | 9.9.9.9 |
| Europe | AdGuard | 94.140.14.14 |
| Eastern Europe | Yandex | 77.88.8.8 |
| Asia-Pacific | 114DNS | 114.114.114.114 |
| Asia-Pacific | AliDNS | 223.5.5.5 |
| Asia-Pacific | DNSPod | 119.29.29.29 |
| Asia-Pacific | TWNIC | 101.101.101.101 |

## Build and Run

Building, configuration, and every runtime option are unchanged from upstream Routinator. Follow the [Routinator documentation](https://routinator.docs.nlnetlabs.nl/).

The multi-resolver DNS pool itself needs no configuration — it is automatically active for all RRDP fetches.

## How It Works

`MultiIpResolver` implements reqwest's `dns::Resolve`. On each DNS lookup it fires ten concurrent queries via `hickory-resolver`, requesting both A and AAAA records from each upstream. It collects all results, removes duplicates by using a `HashSet`, and returns the full pool to reqwest.

reqwest's connection logic iterates the pool in order, moving to the next candidate on failure, so PoP-level failover is completely transparent to the rest of Routinator.

## Based On

Routinator 0.15.2 by [NLnet Labs](https://nlnetlabs.nl), licensed BSD-3-Clause.
This fork carries the same license.
