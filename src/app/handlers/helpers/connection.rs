//! Server connection discovery and testing.

/// Find the first working server connection by testing ALL connections in PARALLEL.
/// Priority: local non-relay > non-relay > relay
///
/// Always injects localhost candidates (127.0.0.1, localhost) to discover
/// same-machine connections even when Plex doesn't advertise them.
pub async fn find_working_connection(
    server: &crate::plex::models::PlexServer,
    token: &str,
    client_identifier: &str,
) -> Option<String> {
    use futures::future::join_all;
    use std::collections::HashSet;

    let mut prioritized: Vec<(usize, String)> = Vec::new();
    let mut seen_uris: HashSet<String> = HashSet::new();

    for conn in &server.connections {
        let priority = if conn.local && !conn.relay {
            0
        } else if !conn.relay {
            1
        } else {
            2
        };
        if seen_uris.insert(conn.uri.clone()) {
            prioritized.push((priority, conn.uri.clone()));
        }
    }

    // Inject localhost candidates (priority 0 = local)
    // Extract unique ports from existing connections, default to 32400
    let mut ports: HashSet<u16> = HashSet::new();
    ports.insert(32400);
    for conn in &server.connections {
        if let Some(port_str) = conn.uri.rsplit(':').next() {
            if let Ok(port) = port_str.parse::<u16>() {
                ports.insert(port);
            }
        }
    }

    for port in &ports {
        for host in &["127.0.0.1", "localhost"] {
            for scheme in &["http", "https"] {
                let uri = format!("{}://{}:{}", scheme, host, port);
                if seen_uris.insert(uri.clone()) {
                    prioritized.push((0, uri));
                }
            }
        }
    }

    // Inject plain HTTP candidates for LAN IPs extracted from .plex.direct hostnames.
    // The HTTPS .plex.direct connections can have TLS issues with large audio streams
    // (IncompleteBody errors), while plain HTTP to the same IP works reliably.
    for conn in &server.connections {
        if conn.local && conn.uri.contains(".plex.direct") {
            // Extract IP from hostname like "192-168-4-47.xxx.plex.direct"
            if let Some(host_part) = conn.uri.split("//").nth(1) {
                if let Some(ip_dashed) = host_part.split('.').next() {
                    let raw_ip = ip_dashed.replace('-', ".");
                    // Validate it looks like an IP
                    if raw_ip.split('.').count() == 4 && raw_ip.split('.').all(|s| s.parse::<u8>().is_ok()) {
                        for port in &ports {
                            let uri = format!("http://{}:{}", raw_ip, port);
                            if seen_uris.insert(uri.clone()) {
                                prioritized.push((0, uri));
                            }
                        }
                    }
                }
            }
        }
    }

    if prioritized.is_empty() {
        tracing::warn!("No connections available for server {}", server.name);
        return None;
    }

    let token_str = token.to_string();
    let client_id = client_identifier.to_string();
    let futures = prioritized.into_iter().map(|(prio, uri)| {
        let token = token_str.clone();
        let client_id = client_id.clone();
        async move {
            match crate::plex::test_connection(&uri, &token, &client_id).await {
                Ok(()) => {
                    tracing::info!("Connection test succeeded: {} (priority {})", uri, prio);
                    Some((prio, uri))
                }
                Err(e) => {
                    tracing::debug!("Connection test failed for {}: {}", uri, e);
                    None
                }
            }
        }
    });

    let results: Vec<Option<(usize, String)>> = join_all(futures).await;

    let mut successes: Vec<(usize, String)> = results.into_iter().flatten().collect();
    successes.sort_by_key(|(prio, _)| *prio);

    if let Some((prio, url)) = successes.into_iter().next() {
        let prio_name = match prio {
            0 => "local",
            1 => "remote",
            _ => "relay",
        };
        tracing::info!("Selected {} connection: {}", prio_name, url);
        return Some(url);
    }

    tracing::warn!("All connection tests failed for server {}", server.name);
    None
}

/// Find the first working connection across multiple servers.
pub async fn find_working_connection_from_servers(
    servers: &[crate::plex::models::PlexServer],
    token: &str,
    client_identifier: &str,
) -> Option<String> {
    for server in servers {
        if let Some(url) = find_working_connection(server, token, client_identifier).await {
            return Some(url);
        }
    }
    None
}
