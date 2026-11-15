use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

pub fn parse_ip(input: &str) -> Option<IpAddr> {
    let trimmed = input.trim().trim_matches(|c| matches!(c, '"' | '\''));
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(sock) = trimmed.parse::<SocketAddr>() {
        return Some(sock.ip());
    }
    if let Some(stripped) = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return stripped.parse::<IpAddr>().ok();
    }
    None
}

fn is_trusted(ip: IpAddr, trusted: &[IpAddr]) -> bool {
    trusted.contains(&ip)
}

pub fn resolve_client_ip(headers: &HeaderMap, remote_ip: IpAddr, trusted: &[IpAddr]) -> IpAddr {
    if trusted.is_empty() || !is_trusted(remote_ip, trusted) {
        return remote_ip;
    }

    if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let mut chain: Vec<IpAddr> = value.split(',').filter_map(parse_ip).collect();

        chain.push(remote_ip);

        while let Some(last) = chain.last().copied() {
            if is_trusted(last, trusted) {
                chain.pop();
            } else {
                break;
            }
        }

        if let Some(client) = chain.first() {
            return *client;
        }
    }

    if let Some(real_ip) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_ip)
    {
        if !is_trusted(real_ip, trusted) {
            return real_ip;
        }
    }

    remote_ip
}

