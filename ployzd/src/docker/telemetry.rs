//! Docker bridge capacity and host telemetry probes.

use std::time::{SystemTime, UNIX_EPOCH};

use ployz_core::{BridgeEndpointCapacity, ByteCapacity, MachineTelemetry};

use super::{Error, LocalDocker};

impl LocalDocker {
    pub(super) async fn bridge_capacity(&self) -> Result<BridgeEndpointCapacity, Error> {
        let network = self
            .client
            .inspect_network(crate::network::DOCKER_NETWORK_NAME, None)
            .await?;
        let usable_endpoints = usable_endpoint_count(
            network
                .ipam
                .and_then(|ipam| ipam.config)
                .into_iter()
                .flatten()
                .filter_map(|config| config.subnet),
        );
        let attached_endpoints = network
            .containers
            .as_ref()
            .map_or(0, |containers| containers.len() as u64);
        Ok(BridgeEndpointCapacity::new(
            usable_endpoints,
            attached_endpoints,
        ))
    }

    pub(super) async fn telemetry(&self) -> Result<MachineTelemetry, Error> {
        let info = self.client.info().await?;
        let docker_root = info
            .docker_root_dir
            .ok_or(Error::MissingField("Docker root directory"))?;
        let (docker_root_total_bytes, docker_root_free_bytes) = filesystem_space(&docker_root)?;
        let (memory_total_bytes, memory_available_bytes) = memory()?;
        let memory = ByteCapacity::parse(memory_total_bytes, memory_available_bytes)
            .map_err(|_| Error::MissingField("consistent host memory capacity"))?;
        let docker_root = ByteCapacity::parse(docker_root_total_bytes, docker_root_free_bytes)
            .map_err(|_| Error::MissingField("consistent Docker-root capacity"))?;
        Ok(MachineTelemetry::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs()),
            self.managed_container_ids().await?.len() as u64,
            std::thread::available_parallelism().map_or(0, |count| count.get() as u64),
            load_average_milli()?,
            memory,
            docker_root,
        ))
    }
}

fn usable_endpoint_count(subnets: impl IntoIterator<Item = String>) -> u64 {
    subnets
        .into_iter()
        .filter_map(|subnet| subnet.parse::<ipnet::Ipv4Net>().ok())
        .map(|subnet| {
            let addresses = 1_u64 << (32 - subnet.prefix_len());
            // Docker reserves the network, broadcast, and configured gateway.
            addresses.saturating_sub(3)
        })
        .sum()
}

fn filesystem_space(path: &str) -> Result<(u64, u64), Error> {
    let stat = nix::sys::statvfs::statvfs(path).map_err(std::io::Error::other)?;
    Ok((
        stat.blocks().saturating_mul(stat.fragment_size()),
        stat.blocks_available().saturating_mul(stat.fragment_size()),
    ))
}

fn memory() -> Result<(u64, u64), Error> {
    let memory = std::fs::read_to_string("/proc/meminfo")?;
    let value = |key| {
        memory
            .lines()
            .find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|line| line.split_whitespace().next()?.parse::<u64>().ok())
            })
            .map(|kib| kib * 1024)
            .ok_or(Error::MissingField(key))
    };
    Ok((value("MemTotal:")?, value("MemAvailable:")?))
}

fn load_average_milli() -> Result<u64, Error> {
    let load = std::fs::read_to_string("/proc/loadavg")?;
    let load = load
        .split_whitespace()
        .next()
        .ok_or(Error::MissingField("load average"))?;
    Ok((load
        .parse::<f64>()
        .map_err(|_| Error::MissingField("load average"))?
        * 1000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_capacity_comes_from_live_subnets_and_attachments() {
        let attachments = ["endpoint-a", "endpoint-b"];
        let usable = usable_endpoint_count(["10.0.0.0/29".into(), "10.0.1.0/30".into()]);
        let capacity = BridgeEndpointCapacity::new(usable, attachments.len() as u64);

        assert_eq!(capacity.usable_endpoints(), 6);
        assert_eq!(capacity.attached_endpoints(), 2);
        assert_eq!(capacity.free_endpoints(), 4);
    }

    #[test]
    fn a_full_live_bridge_has_no_free_capacity() {
        let attachments = ["a", "b", "c", "d", "e"];
        let usable = usable_endpoint_count(["10.0.0.0/29".into()]);
        let capacity = BridgeEndpointCapacity::new(usable, attachments.len() as u64);

        assert_eq!(capacity.usable_endpoints(), 5);
        assert_eq!(capacity.attached_endpoints(), 5);
        assert_eq!(capacity.free_endpoints(), 0);
    }
}
