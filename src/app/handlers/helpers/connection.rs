//! Server connection discovery and testing.

/// Find the first working server connection by testing ALL connections in PARALLEL.
/// Priority: local non-relay > non-relay > relay
pub async fn find_working_connection(
    server: &crate::api::models::PlexServer,
    token: &str,
    client_identifier: &str,
) -> Option<String> {
    use futures::future::join_all;

    let mut prioritized: Vec<(usize, &str)> = Vec::new();

    for conn in &server.connections {
        let priority = if conn.local && !conn.relay {
            0
        } else if !conn.relay {
            1
        } else {
            2
        };
        prioritized.push((priority, conn.uri.as_str()));
    }

    if prioritized.is_empty() {
        tracing::warn!("No connections available for server {}", server.name);
        return None;
    }

    let token_str = token.to_string();
    let client_id = client_identifier.to_string();
    let futures = prioritized.iter().map(|(priority, uri)| {
        let uri = uri.to_string();
        let token = token_str.clone();
        let client_id = client_id.clone();
        let prio = *priority;
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
    servers: &[crate::api::models::PlexServer],
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
