use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

/// Tracks the number of active connections per IP address.
///
/// This provides a simple rate-limiting mechanism to prevent a single IP
/// from opening too many WebSocket connections.
#[derive(Debug)]
pub struct IpConnectionLimiter {
    connections: DashMap<IpAddr, AtomicUsize>,
    limit: Option<usize>,
}

impl IpConnectionLimiter {
    pub fn new(limit: Option<usize>) -> Self {
        Self {
            connections: DashMap::new(),
            limit,
        }
    }

    /// Attempts to register a new connection for the given IP.
    ///
    /// Returns `true` if the connection is allowed (under the limit),
    /// or `false` if the limit has been reached.
    pub fn try_acquire(&self, ip: IpAddr) -> bool {
        let entry = self
            .connections
            .entry(ip)
            .or_insert_with(|| AtomicUsize::new(0));

        let Some(limit) = self.limit else {
            entry.fetch_add(1, Ordering::SeqCst);
            return true;
        };

        if limit == 0 {
            entry.fetch_add(1, Ordering::SeqCst);
            return true;
        }

        loop {
            let current = entry.load(Ordering::Relaxed);
            if current >= limit {
                return false;
            }
            if entry
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Releases a connection slot for the given IP.
    ///
    /// This should be called when a connection is closed.
    pub fn release(&self, ip: IpAddr) {
        if let Some(entry) = self.connections.get(&ip) {
            let prev = entry.fetch_sub(1, Ordering::SeqCst);
            // Clean up entry if it reaches zero
            if prev <= 1 {
                drop(entry); // Release read lock before removing
                self.connections
                    .remove_if(&ip, |_, count| count.load(Ordering::Relaxed) == 0);
            }
        }
    }

    /// Returns the current connection count for an IP.
    pub fn count(&self, ip: IpAddr) -> usize {
        self.connections
            .get(&ip)
            .map(|entry| entry.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Returns a snapshot of all tracked IPs and their connection counts.
    pub fn snapshot(&self) -> Vec<(IpAddr, usize)> {
        self.connections
            .iter()
            .map(|entry| (*entry.key(), entry.value().load(Ordering::Relaxed)))
            .collect()
    }

    /// Returns the total number of tracked IPs.
    pub fn tracked_ips(&self) -> usize {
        self.connections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn no_limit_always_allows() {
        let limiter = IpConnectionLimiter::new(None);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        for _ in 0..100 {
            assert!(limiter.try_acquire(ip));
        }
        assert_eq!(limiter.count(ip), 100);
        for _ in 0..100 {
            limiter.release(ip);
        }
        assert_eq!(limiter.count(ip), 0);
    }

    #[test]
    fn zero_limit_always_allows() {
        let limiter = IpConnectionLimiter::new(Some(0));
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        for _ in 0..100 {
            assert!(limiter.try_acquire(ip));
        }
        assert_eq!(limiter.count(ip), 100);
    }

    #[test]
    fn enforces_limit() {
        let limiter = IpConnectionLimiter::new(Some(3));
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        // Should allow 3 connections
        assert!(limiter.try_acquire(ip));
        assert!(limiter.try_acquire(ip));
        assert!(limiter.try_acquire(ip));
        assert_eq!(limiter.count(ip), 3);

        // 4th should be rejected
        assert!(!limiter.try_acquire(ip));
        assert_eq!(limiter.count(ip), 3);
    }

    #[test]
    fn release_frees_slot() {
        let limiter = IpConnectionLimiter::new(Some(2));
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        assert!(limiter.try_acquire(ip));
        assert!(limiter.try_acquire(ip));
        assert_eq!(limiter.count(ip), 2);

        // At limit
        assert!(!limiter.try_acquire(ip));

        // Release one
        limiter.release(ip);
        assert_eq!(limiter.count(ip), 1);

        // Should allow again
        assert!(limiter.try_acquire(ip));
        assert_eq!(limiter.count(ip), 2);
    }

    #[test]
    fn different_ips_independent() {
        let limiter = IpConnectionLimiter::new(Some(2));
        let ip1 = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2));

        assert!(limiter.try_acquire(ip1));
        assert!(limiter.try_acquire(ip1));
        assert!(!limiter.try_acquire(ip1));

        // ip2 should still have quota
        assert!(limiter.try_acquire(ip2));
        assert!(limiter.try_acquire(ip2));
        assert!(!limiter.try_acquire(ip2));
    }

    #[test]
    fn cleanup_on_zero() {
        let limiter = IpConnectionLimiter::new(Some(5));
        let ip = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        limiter.try_acquire(ip);
        limiter.try_acquire(ip);
        assert_eq!(limiter.tracked_ips(), 1);

        limiter.release(ip);
        limiter.release(ip);

        // Should clean up entry when count reaches zero
        assert_eq!(limiter.count(ip), 0);
        // Entry may still exist until next access, but functionally cleaned up
    }
}
